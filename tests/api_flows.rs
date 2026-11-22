//! API-level regression tests for the beta-hardening endpoints, served on the
//! real composed router over loopback:
//!   * `POST /api/v1/invoices` idempotency per `order_id`
//!   * `GET /api/v1/unmatched-payments` + `POST .../resolve` (store-scoped)
//!
//! Pattern copied from tests/routes_wiring.rs. If you add API behaviour,
//! extend this file rather than growing routes_wiring.rs.

use std::sync::Arc;

use tokio::net::TcpListener;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::invoice;
use easy_bitcoin_payments::rates::RateService;
use easy_bitcoin_payments::routes::{router, AppState};
use easy_bitcoin_payments::store;
use easy_bitcoin_payments::util::now_unix;
use easy_bitcoin_payments::wallet::StubWallet;
use easy_bitcoin_payments::webhook::WebhookEmitter;

/// Sign in as the shop owner (admin page) and return (cookie, csrf). Owner-only
/// actions (webhook settings, key rotation, marking payments handled) need this;
/// the shop's API key is deliberately not enough.
async fn owner_session(base: &str, state: &AppState) -> (String, String) {
    state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::set_password(c, "a good long password", 1_000))
        .await
        .unwrap();
    let login = reqwest::Client::new()
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .json(&serde_json::json!({"password": "a good long password"}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    (cookie, csrf)
}

/// Serve the router with setup marked complete; returns (base url, state,
/// store id, api key).
async fn serve() -> (String, AppState, String, String) {
    serve_with(Config::default()).await
}

async fn serve_with(config: Config) -> (String, AppState, String, String) {
    serve_with_rates(config, RateService::new()).await
}

/// Like `serve_with`, with a Bitcoin price already in place (as a refresh would leave it).
async fn serve_with_rates(
    config: Config,
    rates: RateService,
) -> (String, AppState, String, String) {
    let config = Arc::new(config);
    let db = Db::open(":memory:").unwrap();
    let (shop, api_key, _) = db
        .call(|conn| store::create(conn, "Flow Shop", None))
        .await
        .unwrap();
    db.call(|conn| {
        easy_bitcoin_payments::settings::set(conn, "setup_wallet_created", "true")?;
        easy_bitcoin_payments::settings::set(conn, "setup_backup_confirmed", "true")?;
        easy_bitcoin_payments::settings::set(conn, "setup_token_verified", "true")?;
        easy_bitcoin_payments::settings::set(conn, "setup_complete", "true")
    })
    .await
    .unwrap();
    let state = AppState {
        db,
        config,
        rates: Arc::new(rates),
        wallet: Arc::new(StubWallet::new()),
        webhooks: Arc::new(WebhookEmitter::new(5)),
    };
    let app = router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, state, shop.id, api_key)
}

async fn create(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    order: &str,
    sats: i64,
) -> (u16, serde_json::Value) {
    let resp = client
        .post(format!("{base}/api/v1/invoices"))
        .header("host", "localhost")
        .bearer_auth(key)
        .json(&serde_json::json!({ "order_id": order, "sats": sats }))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap())
}

#[tokio::test]
async fn invoice_creation_is_idempotent_per_order_and_amount() {
    let (base, _state, _store, key) = serve().await;
    let client = reqwest::Client::new();

    let (s1, first) = create(&client, &base, &key, "wc-77", 5_000).await;
    assert_eq!(s1, 201);

    // Same order, same amount: the existing open invoice, with 200.
    let (s2, again) = create(&client, &base, &key, "wc-77", 5_000).await;
    assert_eq!(s2, 200);
    assert_eq!(first["id"], again["id"]);

    // Same order, different amount (cart changed): a new invoice.
    let (s3, changed) = create(&client, &base, &key, "wc-77", 6_000).await;
    assert_eq!(s3, 201);
    assert_ne!(first["id"], changed["id"]);

    // Different order: always new.
    let (s4, other) = create(&client, &base, &key, "wc-78", 5_000).await;
    assert_eq!(s4, 201);
    assert_ne!(first["id"], other["id"]);
}

#[tokio::test]
async fn a_retry_after_a_part_payment_gets_the_underpaid_invoice_back() {
    let (base, state, _store, key) = serve().await;
    let client = reqwest::Client::new();

    let (s1, first) = create(&client, &base, &key, "wc-part", 5_000).await;
    assert_eq!(s1, 201);
    let id = first["id"].as_str().unwrap().to_string();
    let part = id.clone();
    state
        .db
        .call(move |conn| {
            conn.execute(
                "UPDATE invoices SET status = 'underpaid', paid_sats = 2000 WHERE id = ?1",
                [&part],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // The shop asks again for the same order: the same invoice, still taking
    // the rest, not a second full-price one.
    let (s2, again) = create(&client, &base, &key, "wc-part", 5_000).await;
    assert_eq!(s2, 200);
    assert_eq!(again["id"].as_str(), Some(id.as_str()));
    assert_eq!(again["status"], "underpaid");
    let count: i64 = state
        .db
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(count, 1);

    // A changed amount is still a new invoice, as before.
    let (s3, changed) = create(&client, &base, &key, "wc-part", 6_000).await;
    assert_eq!(s3, 201);
    assert_ne!(changed["id"].as_str(), Some(id.as_str()));
}

#[tokio::test]
async fn the_same_order_asked_for_at_the_same_moment_makes_one_invoice() {
    let (base, state, _store, key) = serve().await;
    let client = reqwest::Client::new();

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (client, base, key) = (client.clone(), base.clone(), key.clone());
        tasks.push(tokio::spawn(async move {
            create(&client, &base, &key, "wc-double-click", 5_000).await
        }));
    }
    let mut ids = std::collections::HashSet::new();
    let mut created = 0;
    for task in tasks {
        let (status, body) = task.await.unwrap();
        assert!(status == 200 || status == 201, "status {status}: {body}");
        if status == 201 {
            created += 1;
        }
        ids.insert(body["id"].as_str().unwrap().to_string());
    }
    assert_eq!(created, 1, "exactly one request creates the invoice");
    assert_eq!(ids.len(), 1, "everyone is handed the same invoice");
    let count: i64 = state
        .db
        .call(|c| Ok(c.query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn unmatched_payments_can_be_listed_and_resolved_by_their_store_only() {
    let (base, state, store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;

    // An expired invoice that then received a late payment.
    state
        .db
        .call(move |conn| {
            invoice::insert(
                conn,
                &invoice::NewInvoice {
                    id: "inv_late".to_string(),
                    store_id,
                    order_id: Some("wc-late".to_string()),
                    amount_sats: 1000,
                    fiat_currency: None,
                    fiat_amount: None,
                    rate: None,
                    rate_source: None,
                    rate_locked_at: None,
                    ark_address: "ark1qlate".to_string(),
                    expires_at: now_unix() - 10,
                    confs_required: 1,
                    metadata_json: "{}".to_string(),
                },
            )?;
            invoice::expire_if_due(conn, "inv_late")?;
            invoice::record_unmatched(
                conn,
                &invoice::UnmatchedInput {
                    address: "ark1qlate",
                    rail: "ark",
                    amount_sats: 1000,
                    vtxo_id: Some("v_late"),
                    txid: None,
                    detected_at: 1,
                    reason_override: None,
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();

    // Unauthenticated: rejected.
    let anon = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);
    // The shop's key cannot read it either: only the owner (admin page) can.
    let by_shop_key = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(by_shop_key.status(), 401);

    let rows: Vec<serde_json::Value> = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["order_id"], "wc-late");
    assert_eq!(rows[0]["reason"], "invoice_expired");
    assert!(rows[0]["resolved_at"].is_null());

    let id = rows[0]["id"].as_str().unwrap().to_string();
    // The shop's key cannot hide a payment; only the owner (admin page) can.
    let by_key = client
        .post(format!("{base}/api/v1/unmatched-payments/{id}/resolve"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(by_key.status(), 401);
    let resolved = client
        .post(format!("{base}/api/v1/unmatched-payments/{id}/resolve"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(resolved.status(), 200);

    let rows: Vec<serde_json::Value> = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!rows[0]["resolved_at"].is_null(), "row is marked resolved");

    // A second store's key cannot read or resolve the first store's rows.
    let (_, other_key, _) = state
        .db
        .call(|conn| store::create(conn, "Other Shop", None))
        .await
        .unwrap();
    let theirs = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .bearer_auth(&other_key)
        .send()
        .await
        .unwrap();
    assert_eq!(theirs.status(), 401);
    let cross = client
        .post(format!("{base}/api/v1/unmatched-payments/{id}/resolve"))
        .header("host", "localhost")
        .bearer_auth(&other_key)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 401);
}

#[tokio::test]
async fn setup_token_guards_wizard_writes_but_not_reads() {
    let (base, _state, _store, _key) = serve_with(Config {
        setup_token: Some("s3cret".to_string()),
        ..Config::default()
    })
    .await;
    let client = reqwest::Client::new();

    // Reads stay open so the wizard page can load and show progress.
    let status = client
        .get(format!("{base}/setup/status"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), 200);

    // Writes without the token: 401 setup_token_required.
    let denied = client
        .post(format!("{base}/setup/wallet/mnemonic"))
        .header("host", "localhost")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let body: serde_json::Value = denied.json().await.unwrap();
    assert_eq!(body["error"], "setup_token_required");

    // Wrong token: same.
    let wrong = client
        .post(format!("{base}/setup/wallet/mnemonic"))
        .header("host", "localhost")
        .header("x-ebp-setup-token", "nope")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    // Right token: the guard lets it through (the handler then answers on
    // its own terms; here setup is already complete, so not 401).
    let ok = client
        .post(format!("{base}/setup/wallet/mnemonic"))
        .header("host", "localhost")
        .header("x-ebp-setup-token", "s3cret")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_ne!(ok.status(), 401);
}

#[tokio::test]
async fn the_transfers_file_lists_transfers_and_upkeep_oldest_first_and_needs_the_admin() {
    let client = reqwest::Client::new();
    let (base, state, _store, _key) = serve_with(Config {
        admin_token: Some("adm1n".to_string()),
        ..Config::default()
    })
    .await;
    state
        .db
        .call(|c| {
            for (id, kind, sats, dest, status, detail, at) in [
                ("t2", "sweep_ark", 150_000_i64, "ark1testdestination", "broadcast", None, 2_000_i64),
                ("t1", "renew", 4_000, "ark1testdestination", "broadcast", None, 1_000),
                ("t3", "sweep_onchain", 60_000, "bc1qtestdestination", "failed", Some("=HYPERLINK(\"x\")"), 3_000),
            ] {
                c.execute(
                    "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    rusqlite::params![id, kind, sats, dest, status, detail, at],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

    let no = client
        .get(format!("{base}/api/v1/admin/settlements.csv"))
        .header("host", "localhost")
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(no.status(), 401);

    let res = client
        .get(format!("{base}/api/v1/admin/settlements.csv"))
        .header("host", "localhost")
        .bearer_auth("adm1n")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert!(res.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/csv"));
    assert_eq!(res.headers()["cache-control"], "no-store");
    let csv = res.text().await.unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert!(lines[0].ends_with("date,type,group,amount_sats,status,destination,note"));
    assert_eq!(lines.len(), 4, "a header and three rows: {csv}");
    assert!(lines[1].contains("Routine wallet upkeep,routine upkeep,4000,broadcast"));
    assert!(
        lines[2].contains("Transfer to your wallet (Ark),transfer to your wallet,150000,broadcast")
    );
    assert!(lines[3].contains("Transfer to your wallet (on-chain)"));
    assert!(
        !csv.contains(",=HYPERLINK") && !csv.contains("\"=HYPERLINK"),
        "a formula in a note is defused: {csv}"
    );
}

#[tokio::test]
async fn admin_status_needs_the_admin_token_and_is_hidden_without_one() {
    let client = reqwest::Client::new();

    // No admin token configured: the endpoint does not exist.
    let (base, _s, _store, _key) = serve().await;
    let hidden = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth("anything")
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);

    // Configured: wrong token 401, right token 200 with counters even though
    // barkd is not configured (wallet section just reports an error).
    let (base, _s, _store, store_key) = serve_with(Config {
        admin_token: Some("adm1n".to_string()),
        ..Config::default()
    })
    .await;
    let wrong = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth("nope")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
    // A store API key is not an admin credential.
    let as_store = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth(&store_key)
        .send()
        .await
        .unwrap();
    assert_eq!(as_store.status(), 401);

    let ok: serde_json::Value = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth("adm1n")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ok["counters"]["unmatched_payments_open"], 0);
    assert_eq!(ok["counters"]["webhooks_failed"], 0);
    assert!(ok["wallet"]["error"].is_string());
    // The plain-language checklist: no wallet configured and a plain-http
    // address are both reported as problems, worst first.
    let checks = ok["checks"].as_array().expect("checks array");
    assert_eq!(checks[0]["level"], "bad");
    let level_of = |id: &str| {
        checks
            .iter()
            .find(|c| c["id"] == id)
            .map(|c| c["level"].clone())
    };
    assert_eq!(level_of("wallet"), Some(serde_json::json!("bad")));
    // The test harness serves on localhost: flagged as a warning, not an alarm.
    assert_eq!(level_of("https"), Some(serde_json::json!("warn")));
}

#[tokio::test]
async fn there_is_no_on_chain_way_to_pay_and_no_combined_code() {
    let client = reqwest::Client::new();
    let (base, _s, _store, key) = serve().await;
    let (_, inv) = create(&client, &base, &key, "wc-oc", 2_000).await;
    // The API and the checkout page's data carry an Ark address (and a Lightning
    // invoice when there is one), and nothing about regular Bitcoin.
    assert!(inv["ark_address"].is_string());
    assert!(inv.get("onchain_address").is_none());
    let id = inv["id"].as_str().unwrap();
    let co: serde_json::Value = client
        .get(format!("{base}/i/{id}/json"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(co["ark_address"].is_string());
    for gone in ["onchain_enabled", "onchain_address", "bip321"] {
        assert!(co.get(gone).is_none(), "{gone} must not be offered");
    }
    let st: serde_json::Value = client
        .get(format!("{base}/i/{id}/status"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    for gone in ["onchain_confirmations", "onchain_confirmations_required"] {
        assert!(st.get(gone).is_none(), "{gone} must not be offered");
    }
}

#[tokio::test]
async fn every_response_carries_browser_hardening_headers() {
    let (base, _s, _store, _key) = serve().await;
    let client = reqwest::Client::new();
    // A page that shows secrets, an API route, and a 404 all get them.
    for path in [
        "/setup",
        "/setup/status",
        "/api/v1/health",
        "/no/such/route",
    ] {
        let resp = client
            .get(format!("{base}{path}"))
            .header("host", "localhost")
            .send()
            .await
            .unwrap();
        let h = resp.headers();
        assert_eq!(h["x-frame-options"], "DENY", "{path}");
        assert_eq!(h["x-content-type-options"], "nosniff", "{path}");
        assert_eq!(h["referrer-policy"], "no-referrer", "{path}");
        assert!(
            h["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn webhook_config_and_key_rotation() {
    let (base, state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let put = |body: serde_json::Value, key: String| {
        let client = client.clone();
        let base = base.clone();
        let (cookie, csrf) = (cookie.clone(), csrf.clone());
        async move {
            // `key` is the shop's API key; webhook settings ignore it and need
            // the owner's session.
            let _ = key;
            client
                .put(format!("{base}/api/v1/stores/webhook"))
                .header("host", "localhost")
                .header("cookie", cookie)
                .header("x-ebp-csrf", csrf)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

    // A shop's API key cannot change where notifications go, or replace itself.
    let by_key = client
        .put(format!("{base}/api/v1/stores/webhook"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .json(&serde_json::json!({ "url": "https://evil.example.com/x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(by_key.status(), 401);
    let by_key = client
        .post(format!("{base}/api/v1/stores/rotate-key"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(by_key.status(), 401);

    // Unsafe targets are refused.
    let bad = put(
        serde_json::json!({ "url": "http://169.254.169.254/x" }),
        key.clone(),
    )
    .await;
    assert_eq!(bad.status(), 400);

    // First valid URL issues a secret, shown once.
    let ok = put(
        serde_json::json!({ "url": "https://shop.example.com/hook" }),
        key.clone(),
    )
    .await;
    assert_eq!(ok.status(), 200);
    let v: serde_json::Value = ok.json().await.unwrap();
    let first = v["webhook_secret"].as_str().unwrap().to_string();
    assert!(first.starts_with("whsec_"));

    // Changing the URL alone keeps the secret (no new secret returned).
    let same = put(
        serde_json::json!({ "url": "https://shop.example.com/other" }),
        key.clone(),
    )
    .await;
    let v: serde_json::Value = same.json().await.unwrap();
    assert!(v["webhook_secret"].is_null());

    // Explicit rotation issues a different one.
    let rot = put(
        serde_json::json!({ "url": "https://shop.example.com/other", "rotate_secret": true }),
        key.clone(),
    )
    .await;
    let v: serde_json::Value = rot.json().await.unwrap();
    assert_ne!(v["webhook_secret"].as_str().unwrap(), first);

    // Key rotation (by the owner): new key works, old key is dead.
    let r = client
        .post(format!("{base}/api/v1/stores/rotate-key"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let new_key = r.json::<serde_json::Value>().await.unwrap()["api_key"]
        .as_str()
        .unwrap()
        .to_string();
    let list = |k: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .get(format!("{base}/api/v1/invoices?limit=1"))
                .header("host", "localhost")
                .bearer_auth(k)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(list(key).await, 401);
    assert_eq!(list(new_key).await, 200);
}

#[tokio::test]
async fn invoice_inputs_are_bounded_and_unmatched_list_paginates() {
    let (base, state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, _csrf) = owner_session(&base, &state).await;
    let long = "x".repeat(129);
    let (status, _) = create(&client, &base, &key, &long, 1000).await;
    assert_eq!(status, 400, "order_id over 128 bytes is rejected");

    let big = serde_json::json!({ "pad": "y".repeat(5000) });
    let resp = client
        .post(format!("{base}/api/v1/invoices"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .json(&serde_json::json!({ "order_id": "ok", "sats": 1000, "metadata": big }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "metadata over 4096 bytes is rejected");

    let page = client
        .get(format!("{base}/api/v1/unmatched-payments?limit=1&offset=0"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
}

#[tokio::test]
async fn one_payment_request_cannot_exceed_the_servers_size_limit() {
    let (base, _state, _store_id, key) = serve_with(Config {
        max_invoice_sats: 50_000,
        ..Config::default()
    })
    .await;
    let client = reqwest::Client::new();
    let (status, ok) = create(&client, &base, &key, "at-limit", 50_000).await;
    assert_eq!(status, 201, "exactly the limit is fine: {ok}");
    let (status, body) = create(&client, &base, &key, "over-limit", 50_001).await;
    assert_eq!(status, 400, "{body}");
    let msg = body.to_string();
    assert!(
        msg.contains("larger than this server accepts") && msg.contains("EBP_MAX_INVOICE_SATS"),
        "plain words plus the knob: {msg}"
    );
}

#[tokio::test]
async fn one_shop_key_cannot_flood_the_server_with_payment_requests() {
    let (base, _state, _store_id, key) = serve_with(Config {
        invoice_rate_limit_per_min: 3,
        ..Config::default()
    })
    .await;
    let client = reqwest::Client::new();
    for i in 0..3 {
        let (status, body) = create(&client, &base, &key, &format!("burst-{i}"), 1000).await;
        assert_eq!(status, 201, "request {i}: {body}");
    }
    let resp = client
        .post(format!("{base}/api/v1/invoices"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .json(&serde_json::json!({ "order_id": "burst-3", "sats": 1000 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limited", "{body}");

    // Another shop is not slowed down by this one.
    let (other_base, _s, _id, other_key) = serve_with(Config {
        invoice_rate_limit_per_min: 3,
        ..Config::default()
    })
    .await;
    let (status, _) = create(&client, &other_base, &other_key, "fine", 1000).await;
    assert_eq!(status, 201);

    // 0 turns the limit off.
    let (base0, _s0, _id0, key0) = serve_with(Config {
        invoice_rate_limit_per_min: 0,
        ..Config::default()
    })
    .await;
    for i in 0..8 {
        let (status, _) = create(&client, &base0, &key0, &format!("free-{i}"), 1000).await;
        assert_eq!(status, 201);
    }
}

#[tokio::test]
async fn qr_script_is_served_same_origin_and_checkout_uses_it() {
    let (base, _state, _store_id, _key) = serve().await;
    let client = reqwest::Client::new();
    let js = client
        .get(format!("{base}/assets/qr.js"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(js.status(), 200);
    assert!(js.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/javascript"));
    assert!(js.text().await.unwrap().contains("ebpQrSvg"));
}

/// True if `html` has an inline event handler such as ` onclick="`.
fn has_inline_handler(html: &str) -> bool {
    html.match_indices(" on").any(|(i, _)| {
        let rest = &html[i + 3..];
        let name_len = rest.bytes().take_while(|b| b.is_ascii_lowercase()).count();
        name_len > 0 && rest[name_len..].starts_with("=\"")
    })
}

#[tokio::test]
async fn pages_carry_no_inline_code_and_the_csp_allows_none() {
    let (base, _state, _store_id, _key) = serve().await;
    let client = reqwest::Client::new();

    // The policy no longer allows inline scripts or styles.
    let resp = client
        .get(format!("{base}/api/v1/health"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    let csp = resp.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(!csp.contains("unsafe-inline"), "{csp}");
    assert!(csp.contains("script-src 'self';"), "{csp}");
    assert!(csp.contains("style-src 'self';"), "{csp}");

    // So the pages must not depend on inline code: no <style> blocks, no
    // style="" attributes, no inline event handlers, and every <script> loads
    // a file from this server.
    let pages = [
        ("checkout", include_str!("../assets/checkout.html")),
        ("dashboard", include_str!("../assets/dashboard.html")),
        ("setup", include_str!("../assets/setup.html")),
    ];
    for (page, html) in pages {
        assert!(!html.contains("<style"), "{page}: inline <style>");
        assert!(!html.contains("style=\""), "{page}: style attribute");
        assert!(!has_inline_handler(html), "{page}: inline event handler");
        for tag in html.split("<script").skip(1) {
            let open = tag.split('>').next().unwrap_or("");
            assert!(
                open.contains("src=\"/assets/"),
                "{page}: inline <script>{open}"
            );
        }
        // Its own script and stylesheet are linked and served.
        for (ext, content_type) in [("js", "application/javascript"), ("css", "text/css")] {
            let path = format!("/assets/{page}.{ext}");
            assert!(html.contains(&path), "{page} does not link {path}");
            let r = client
                .get(format!("{base}{path}"))
                .header("host", "localhost")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "{path}");
            assert!(
                r.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with(content_type),
                "{path}"
            );
            assert!(
                !r.text().await.unwrap().trim().is_empty(),
                "{path} is empty"
            );
        }
    }
}

#[tokio::test]
async fn mascot_art_is_built_in_read_only_and_cannot_be_uploaded() {
    let (base, _state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let names = [
        "sir-body",
        "sir-arm",
        "ufo-top",
        "ufo-beam",
        "fancy-body",
        "fancy-hat",
        "acro",
    ];
    let mut originals = Vec::new();
    for name in names {
        let r = client
            .get(format!("{base}/assets/mascot/{name}.png"))
            .header("host", "localhost")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{name}");
        assert_eq!(
            r.headers()["content-type"].to_str().unwrap(),
            "image/png",
            "{name}"
        );
        let body = r.bytes().await.unwrap();
        assert!(
            body.starts_with(&[0x89, b'P', b'N', b'G']),
            "{name} is a PNG"
        );
        originals.push(body);
    }
    // Only the fixed names exist; anything else (odd spellings, path tricks) is a plain 404.
    for bad in [
        "nope.png",
        "sir-body",
        "SIR-BODY.PNG",
        "..%2Fqr.js",
        "%2e%2e%2f%2e%2e%2fCargo.toml",
        "sir-body.png%00",
    ] {
        let r = client
            .get(format!("{base}/assets/mascot/{bad}"))
            .header("host", "localhost")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{bad}");
    }
    // Nothing can add or replace an image, even with the store's own API key.
    for method in [
        reqwest::Method::POST,
        reqwest::Method::PUT,
        reqwest::Method::PATCH,
        reqwest::Method::DELETE,
    ] {
        for target in ["sir-body.png", "brand-new.png"] {
            let r = client
                .request(method.clone(), format!("{base}/assets/mascot/{target}"))
                .header("host", "localhost")
                .bearer_auth(&key)
                .body(vec![0x89, b'P', b'N', b'G'])
                .send()
                .await
                .unwrap();
            assert!(
                r.status() == 405 || r.status() == 404,
                "{method} {target} gave {}",
                r.status()
            );
        }
    }
    // The pictures are unchanged afterwards, and no new name appeared.
    for (name, before) in names.iter().zip(originals) {
        let r = client
            .get(format!("{base}/assets/mascot/{name}.png"))
            .header("host", "localhost")
            .send()
            .await
            .unwrap();
        assert_eq!(r.bytes().await.unwrap(), before, "{name} changed");
    }
    let r = client
        .get(format!("{base}/assets/mascot/brand-new.png"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn delivery_log_resend_and_audit_trail_are_store_scoped() {
    let (base, state, store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    // A failed delivery for this store and one for another store.
    state
        .db
        .call({
            let store_id = store_id.clone();
            move |conn| {
                for (id, sid) in [("whd_mine", store_id.as_str()), ("whd_other", "store_other")] {
                    conn.execute(
                        "INSERT INTO webhook_deliveries (id, store_id, invoice_id, event, idempotency_key, payload_json, status, attempts, next_attempt_at, last_error, created_at)
                         VALUES (?1, ?2, 'inv_x', 'invoice.paid', ?1, '{}', 'failed', 6, 0, 'boom', 1)",
                        rusqlite::params![id, sid],
                    )?;
                }
                Ok(())
            }
        })
        .await
        .unwrap();

    let list: serde_json::Value = client
        .get(format!("{base}/api/v1/webhook-deliveries?status=failed"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1, "only this store's rows");
    for path in [
        "webhook-deliveries",
        "audit-log",
        "unmatched-payments",
        "payout",
    ] {
        let by_key = client
            .get(format!("{base}/api/v1/{path}"))
            .header("host", "localhost")
            .bearer_auth(&key)
            .send()
            .await
            .unwrap();
        assert_eq!(by_key.status(), 401, "the shop key cannot read {path}");
    }
    assert_eq!(list[0]["id"], "whd_mine");

    let other = client
        .post(format!("{base}/api/v1/webhook-deliveries/whd_other/resend"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), 404, "another store's delivery is invisible");

    let ok = client
        .post(format!("{base}/api/v1/webhook-deliveries/whd_mine/resend"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let pending: serde_json::Value = client
        .get(format!("{base}/api/v1/webhook-deliveries?status=pending"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pending[0]["attempts"], 0);

    let audit: serde_json::Value = client
        .get(format!("{base}/api/v1/audit-log"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // (Newest first, but the owner's sign-in can share the same second.)
    assert!(audit
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["action"] == "webhook.resend"));
}

#[tokio::test]
async fn generated_setup_code_is_typo_tolerant_and_wrong_codes_are_refused() {
    let config = Config {
        setup_token: Some("K7QM-2XPD-9TWE".to_string()),
        setup_token_generated: true,
        ..Config::default()
    };
    let (base, _state, _store_id, _key) = serve_with(config).await;
    let client = reqwest::Client::new();
    let verify = |code: &'static str| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}/setup/verify-code"))
                .header("host", "localhost")
                .header("x-ebp-setup-token", code)
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    assert_eq!(verify("K7QM-2XPD-9TWE").await, 200);
    assert_eq!(
        verify(" k7qm 2xpd 9twe ").await,
        200,
        "case/space/dash tolerant"
    );
    assert_eq!(verify("K7QM-2XPD-9TWF").await, 401);
    assert_eq!(verify("").await, 401);
}

#[tokio::test]
async fn setup_writes_are_refused_over_plain_http_on_a_public_address() {
    let config = Config {
        public_base_url: "http://pay.example.com".to_string(),
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    // The harness finishes setup; reopen it so the wizard status applies.
    state
        .db
        .call(|c| easy_bitcoin_payments::settings::set(c, "setup_complete", "false"))
        .await
        .unwrap();
    let client = reqwest::Client::new();

    let status: serde_json::Value = client
        .get(format!("{base}/setup/status"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["https_required"], true);

    let resp = client
        .post(format!("{base}/setup/verify-code"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "https_required");
}

#[tokio::test]
async fn admin_login_session_csrf_and_logout() {
    let config = Config {
        password_hash_iterations: 1_000,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::set_password(c, "a good long password", 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let get = |path: &str, cookie: Option<String>| {
        let mut r = client
            .get(format!("{base}{path}"))
            .header("host", "localhost");
        if let Some(c) = cookie {
            r = r.header("cookie", c);
        }
        r.send()
    };

    // Not signed in: nothing works, and the page reports it.
    assert_eq!(
        get("/api/v1/audit-log", None)
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    let s: serde_json::Value = get("/admin/session", None)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["password_set"], true);
    assert_eq!(s["authenticated"], false);

    // Wrong password.
    let bad = client
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .json(&serde_json::json!({"password": "wrong wrong wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 401);

    // Right password: cookie is HttpOnly + SameSite=Strict.
    let ok = client
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .json(&serde_json::json!({"password": "a good long password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    let set_cookie = ok.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
    let cookie = set_cookie.split(';').next().unwrap().to_string();
    let body: serde_json::Value = ok.json().await.unwrap();
    let csrf = body["csrf"].as_str().unwrap().to_string();

    // Reads work with the cookie alone.
    assert_eq!(
        get("/api/v1/audit-log", Some(cookie.clone()))
            .await
            .unwrap()
            .status()
            .as_u16(),
        200
    );

    // Writes need the CSRF header.
    let no_csrf = client
        .post(format!("{base}/api/v1/stores/rotate-key"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status().as_u16(), 401);
    let with_csrf = client
        .post(format!("{base}/api/v1/stores/rotate-key"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(with_csrf.status().as_u16(), 200);
    let new_key: serde_json::Value = with_csrf.json().await.unwrap();
    assert!(new_key["api_key"].as_str().unwrap().len() > 20);

    // Logout kills the session.
    let out = client
        .post(format!("{base}/admin/logout"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(out.status().as_u16(), 200);
    assert_eq!(
        get("/api/v1/audit-log", Some(cookie))
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
}

#[tokio::test]
async fn wizard_store_step_sets_the_admin_password() {
    let config = Config {
        password_hash_iterations: 1_000,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::settings::set(c, "setup_complete", "false"))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let post = |body: serde_json::Value| {
        client
            .post(format!("{base}/setup/store"))
            .header("host", "localhost")
            .json(&body)
            .send()
    };

    // Too short: refused, nothing set.
    let r = post(serde_json::json!({"name": "Shop", "admin_password": "short", "payout_address": "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let set = state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::password_is_set(c))
        .await
        .unwrap();
    assert!(!set);

    // No payout address: refused, nothing set.
    let r = post(serde_json::json!({"name": "Shop", "admin_password": "a good long password"}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let set = state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::password_is_set(c))
        .await
        .unwrap();
    assert!(!set);

    // Good password and address: stored hashed, and it logs in.
    let r = post(serde_json::json!({"name": "Shop", "admin_password": "a good long password", "payout_address": "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let login = client
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .json(&serde_json::json!({"password": "a good long password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status().as_u16(), 200);
}

#[tokio::test]
async fn backup_cannot_be_confirmed_without_verifying_the_words() {
    let (base, state, _store_id, _key) = serve_with(Config::default()).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::settings::set(c, "setup_complete", "false"))
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .post(format!("{base}/setup/wallet/confirm-backup"))
        .header("host", "localhost")
        .json(&serde_json::json!({"confirmed": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 409);
}

#[tokio::test]
async fn webhook_test_needs_a_url_and_queues_a_test_event() {
    let (base, state, _store_id, _key) = serve_with(Config::default()).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let test = || {
        client
            .post(format!("{base}/api/v1/stores/webhook/test"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .header("x-ebp-csrf", &csrf)
            .send()
    };
    // No webhook yet: refused with a plain message.
    assert_eq!(test().await.unwrap().status().as_u16(), 400);

    let set = client
        .put(format!("{base}/api/v1/stores/webhook"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .json(&serde_json::json!({ "url": "https://shop.example.com/hook" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status().as_u16(), 200);
    assert_eq!(test().await.unwrap().status().as_u16(), 200);

    let list: serde_json::Value = client
        .get(format!("{base}/api/v1/webhook-deliveries"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["event"] == "webhook.test"));
}

#[tokio::test]
async fn version_endpoint_needs_auth_and_respects_the_off_switch() {
    let client = reqwest::Client::new();
    let (base, state, shop_id, key) = serve_with(Config {
        update_check: false,
        ..Config::default()
    })
    .await;

    let anon = client
        .get(format!("{base}/api/v1/admin/version"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);

    // One open order counts toward "best time to update".
    let (st, _) = create(&client, &base, &key, "ver-1", 1000).await;
    assert_eq!(st, 201);
    let _ = (&state, &shop_id);

    let v: serde_json::Value = client
        .get(format!("{base}/api/v1/admin/version"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["check_enabled"], false);
    assert!(v["latest"].is_null(), "no lookup when the check is off");
    assert_eq!(v["update_available"], false);
    assert_eq!(v["open_orders"], 1);
    assert!(v["host"].is_string());
}

/// The server prepares one backup at a time (process-wide), so tests that
/// download one must not overlap.
static BACKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn a_downloaded_backup_can_be_checked_from_the_admin_page() {
    let _one_backup_at_a_time = BACKUP_LOCK.lock().await;
    // The check route sits behind the password brake (a few tries a minute per
    // client), so each check below comes from its own address.
    let config = Config {
        password_hash_iterations: 1_000,
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, key) = serve_with(config).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    // owner_session signs in with this password.
    let pw = "a good long password";
    let dl = client
        .post(format!("{base}/admin/backup"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .json(&serde_json::json!({ "admin_password": pw }))
        .send()
        .await
        .unwrap();
    assert_eq!(dl.status().as_u16(), 200);
    let file = dl.bytes().await.unwrap().to_vec();
    assert!(
        file.len() > 64 * 1024,
        "the sample backup is bigger than the 64 KB default upload limit ({} bytes), so this test proves the larger limit applies",
        file.len()
    );
    let body = |password: &str, bytes: &[u8]| {
        let mut v = (password.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(password.as_bytes());
        v.extend_from_slice(bytes);
        v
    };
    let next_client = std::cell::Cell::new(0u32);
    let check = |csrf: Option<&str>, cookie: Option<&str>, payload: Vec<u8>| {
        next_client.set(next_client.get() + 1);
        let mut r = client
            .post(format!("{base}/admin/backup/check"))
            .header("host", "localhost")
            .header("x-forwarded-for", format!("10.8.0.{}", next_client.get()))
            .header("content-type", "application/octet-stream")
            .body(payload);
        if let Some(c) = csrf {
            r = r.header("x-ebp-csrf", c);
        }
        if let Some(c) = cookie {
            r = r.header("cookie", c);
        }
        r.send()
    };

    // Good file, right password: names and sizes only.
    let ok = check(Some(&csrf), Some(&cookie), body(pw, &file))
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    let j: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(j["ok"], true);
    assert_eq!(j["has_database"], true);
    assert_eq!(j["has_key"], true);
    assert_eq!(j["wallet_included"], false);
    assert!(
        !j.to_string().contains("BEGIN"),
        "no file contents in the answer"
    );

    // Wrong backup password, and a file that is not a backup: plain 400s.
    let wrong = check(Some(&csrf), Some(&cookie), body("not the password", &file))
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 400);
    assert!(wrong
        .text()
        .await
        .unwrap()
        .contains("wrong backup password"));
    let junk = check(Some(&csrf), Some(&cookie), body(pw, &vec![7u8; 100 * 1024]))
        .await
        .unwrap();
    assert_eq!(junk.status().as_u16(), 400);
    // An upload that is not in the expected shape.
    let odd = check(Some(&csrf), Some(&cookie), vec![1, 2]).await.unwrap();
    assert_eq!(odd.status().as_u16(), 400);

    // Needs the signed-in session and the CSRF header.
    let anon = check(None, None, body(pw, &file)).await.unwrap();
    assert_eq!(anon.status().as_u16(), 401);
    let no_csrf = check(None, Some(&cookie), body(pw, &file)).await.unwrap();
    assert_eq!(no_csrf.status().as_u16(), 401);
    // The shop's API key is not a session.
    let by_key = client
        .post(format!("{base}/admin/backup/check"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .body(body(pw, &file))
        .send()
        .await
        .unwrap();
    assert_eq!(by_key.status().as_u16(), 401);
}

#[tokio::test]
async fn encrypted_backup_download_needs_session_admin_password_and_decrypts() {
    let _one_backup_at_a_time = BACKUP_LOCK.lock().await;
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();
    let config = Config {
        password_hash_iterations: 1_000,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::set_password(c, "a good long password", 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let backup = |csrf: Option<&str>, cookie: Option<&str>, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}/admin/backup"))
            .header("host", "localhost")
            .json(&body);
        if let Some(c) = csrf {
            r = r.header("x-ebp-csrf", c);
        }
        if let Some(c) = cookie {
            r = r.header("cookie", c);
        }
        r.send()
    };
    let good = serde_json::json!({"admin_password": "a good long password"});

    // Not signed in.
    assert_eq!(
        backup(None, None, good.clone())
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );

    let ok = client
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .json(&serde_json::json!({"password": "a good long password"}))
        .send()
        .await
        .unwrap();
    let cookie = ok.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = ok.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();

    // No CSRF header: refused.
    assert_eq!(
        backup(None, Some(&cookie), good.clone())
            .await
            .unwrap()
            .status()
            .as_u16(),
        401
    );
    // A wrong admin password is a 400, not a download.
    let r = backup(
        Some(&csrf),
        Some(&cookie),
        serde_json::json!({"admin_password": "wrong wrong wrong"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    let r = backup(Some(&csrf), Some(&cookie), good).await.unwrap();
    let status = r.status().as_u16();
    if status != 200 {
        panic!("backup failed: {status} {}", r.text().await.unwrap());
    }
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert!(r.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .contains("attachment"));
    assert_eq!(r.headers()["x-ebp-wallet-included"], "false");
    let bytes = r.bytes().await.unwrap();

    // The file is encrypted: no readable store name, no SQLite header.
    assert!(!bytes.windows(9).any(|w| w == b"Flow Shop"));
    assert!(!bytes.windows(15).any(|w| w == b"SQLite format 3"));
    assert!(easy_bitcoin_payments::backup::open(&bytes, "some other password").is_err());
    let entries = easy_bitcoin_payments::backup::open(&bytes, "a good long password").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "ebp.sqlite3");
    // The key rides inside the (encrypted) bundle so one file restores everything.
    assert_eq!(entries[1].name, "ebp.key");
    assert_eq!(
        String::from_utf8(entries[1].data.clone()).unwrap(),
        easy_bitcoin_payments::secrets::get().key_hex()
    );
    assert!(entries[0].data.starts_with(b"SQLite format 3"));
    // The decrypted database has the shop, and no live logins.
    assert!(entries[0].data.windows(9).any(|w| w == b"Flow Shop"));

    // The download is recorded in the security activity.
    let audit: serde_json::Value = client
        .get(format!("{base}/api/v1/audit-log?limit=10"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(audit
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["action"] == "admin.backup_downloaded"));
}

#[tokio::test]
async fn payout_address_changes_wait_and_need_the_password() {
    let config = Config {
        password_hash_iterations: 1_000,
        // Each request below pretends to come from its own address so the
        // password-guessing brake (a few tries a minute per client) does not interfere.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::set_password(c, "a good long password", 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{base}/admin/login"))
        .header("host", "localhost")
        .header("x-forwarded-for", "10.2.1.1")
        .json(&serde_json::json!({"password": "a good long password"}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let n = std::sync::atomic::AtomicU32::new(1);
    let post = |path: &str, body: serde_json::Value| {
        client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header(
                "x-forwarded-for",
                format!(
                    "10.2.0.{}",
                    n.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                ),
            )
            .header("cookie", &cookie)
            .header("x-ebp-csrf", &csrf)
            .json(&body)
            .send()
    };
    let get = || async {
        client
            .get(format!("{base}/api/v1/payout"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()
    };
    let a = "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
    let b = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    assert!(get().await["address"].is_null());

    // Wrong password, and a typo, are refused.
    let r = post(
        "/admin/payout",
        serde_json::json!({"address": a, "admin_password": "nope nope nope"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = post(
        "/admin/payout",
        serde_json::json!({"address": "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsy", "admin_password": "a good long password"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // A first address set here (not in the setup wizard) waits too.
    let r: serde_json::Value = post(
        "/admin/payout",
        serde_json::json!({"address": a, "admin_password": "a good long password"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(r["applied_now"], false);
    assert!(get().await["address"].is_null());
    // The wizard sets the first one at once.
    state
        .db
        .call(move |c| easy_bitcoin_payments::payout::set_initial(c, a, "test"))
        .await
        .unwrap();
    assert_eq!(get().await["address"], a);

    // Changing it: waits, old address stays in use, can be cancelled.
    let r: serde_json::Value = post(
        "/admin/payout",
        serde_json::json!({"address": b, "admin_password": "a good long password"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(r["applied_now"], false);
    let g = get().await;
    assert_eq!(g["address"], a);
    assert_eq!(g["pending_address"], b);

    let r = post("/admin/payout/cancel", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(get().await["pending_address"].is_null());

    // Not signed in: refused.
    let r = client
        .post(format!("{base}/admin/payout"))
        .header("host", "localhost")
        .header("x-forwarded-for", "10.2.1.2")
        .json(&serde_json::json!({"address": a, "admin_password": "a good long password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    let text = |r: reqwest::Response| async move {
        r.json::<serde_json::Value>().await.unwrap().to_string()
    };
    let pw = "a good long password";
    let tb = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    // Nothing stays on the server (the whole balance is sent once the minimum
    // is reached), and there is no setting to keep an amount back. The emergency
    // withdrawal is opt-in.
    let g = get().await;
    assert_eq!(g["kind"], "ark");
    assert!(g.get("keep_sats").is_none(), "there is no amount to keep");
    assert_eq!(g["exit_opted_in"], false);
    let r = post(
        "/admin/payout/cap",
        serde_json::json!({"cap_sats": 50_000, "admin_password": pw}),
    )
    .await
    .unwrap();
    assert!(
        matches!(r.status().as_u16(), 404 | 405),
        "the amount-to-keep route is gone"
    );
    // Turning it on needs two-step sign-in, the password and a code: refused here.
    let r = post(
        "/admin/exit/optin",
        serde_json::json!({"on": true, "admin_password": pw, "code": "123456"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(text(r).await.contains("two-step"));
    assert_eq!(get().await["exit_opted_in"], false);

    // The smallest amount that is sent on: each kind of address has a hard floor
    // (Ark 1,500, on-chain 50,000) and above it the owner decides.
    let g = get().await;
    assert_eq!(g["sweep_min_sats"], 1_500);
    assert_eq!(g["sweep_min_floor_sats"], 1_500);
    assert_eq!(g["sweep_min_is_custom"], false);
    assert_eq!(g["ark_min_floor_sats"], 1_500);
    assert_eq!(g["onchain_min_floor_sats"], 50_000);
    for (kind, bad) in [
        ("ark", 1_499i64),
        ("ark", 1_000),
        ("ark", 0),
        ("ark", -1),
        ("ark", 10_000_001),
        ("onchain", 49_999),
        ("onchain", 1_500),
        ("onchain", 0),
        ("onchain", -1),
        ("onchain", 10_000_001),
    ] {
        let r = post(
            "/admin/payout/min",
            serde_json::json!({"kind": kind, "min_sats": bad, "admin_password": pw}),
        )
        .await
        .unwrap();
        assert_eq!(r.status().as_u16(), 400, "{kind} {bad}");
    }
    // Wrong password and an unknown kind are refused too.
    for body in [
        serde_json::json!({"kind": "ark", "min_sats": 3_000, "admin_password": "nope nope nope"}),
        serde_json::json!({"kind": "lightning", "min_sats": 3_000, "admin_password": pw}),
    ] {
        let r = post("/admin/payout/min", body).await.unwrap();
        assert_eq!(r.status().as_u16(), 400);
    }
    // Under the floor, the answer explains why in plain words.
    let r = post(
        "/admin/payout/min",
        serde_json::json!({"kind": "ark", "min_sats": 1_000, "admin_password": pw}),
    )
    .await
    .unwrap();
    let why = text(r).await;
    assert!(
        why.contains("1,000 sats is lower than the smallest amount we allow"),
        "{why}"
    );
    assert!(why.contains("1,500"), "{why}");
    assert!(why.contains("get stuck"), "{why}");
    let r = post(
        "/admin/payout/min",
        serde_json::json!({"kind": "onchain", "min_sats": 20_000, "admin_password": pw}),
    )
    .await
    .unwrap();
    let why = text(r).await;
    assert!(
        why.contains("20,000 sats is lower than the smallest amount we allow"),
        "{why}"
    );
    assert!(why.contains("50,000"), "{why}");
    assert!(why.contains("network fee"), "{why}");
    assert!(why.contains("Ark payout address"), "{why}");
    assert_eq!(get().await["sweep_min_is_custom"], false);
    // A good value applies at once (no waiting period, unlike the address).
    let r: serde_json::Value = post(
        "/admin/payout/min",
        serde_json::json!({"kind": "ark", "min_sats": 3_000, "admin_password": pw}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(r["sweep_min_sats"], 3_000);
    let g = get().await;
    assert_eq!(g["sweep_min_is_custom"], true);
    assert_eq!(g["sweep_min_sats"], 3_000);
    // The on-chain number is its own: saving it leaves the Ark address's view alone.
    let r = post(
        "/admin/payout/min",
        serde_json::json!({"kind": "onchain", "min_sats": 250_000, "admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(get().await["sweep_min_sats"], 3_000);
    // Exactly the floor is allowed.
    let r = post(
        "/admin/payout/min",
        serde_json::json!({"kind": "ark", "min_sats": 1_500, "admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(get().await["sweep_min_sats"], 1_500);
    // Back to the defaults.
    for kind in ["ark", "onchain"] {
        let r = post(
            "/admin/payout/min",
            serde_json::json!({"kind": kind, "min_sats": null, "admin_password": pw}),
        )
        .await
        .unwrap();
        assert_eq!(r.status().as_u16(), 200);
    }
    assert_eq!(get().await["sweep_min_is_custom"], false);
    // Not signed in: refused.
    for path in ["/admin/payout/min", "/admin/exit/optin"] {
        let r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("x-forwarded-for", "10.2.1.3")
            .json(&serde_json::json!({"kind": "ark", "min_sats": 5000, "on": false, "admin_password": pw}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 401, "{path}");
    }

    // The emergency withdrawal turned on (set directly here: the route needs
    // two-step sign-in, which other tests cover).
    state
        .db
        .call(|c| easy_bitcoin_payments::emergency_exit::set_opted_in(c, true))
        .await
        .unwrap();
    assert_eq!(get().await["exit_opted_in"], true);
    // While it is on, the part of the fee money above 5,000 can still be sent back
    // (worked out in `emergency_exit::leftover_plan`); like every money action it
    // needs two-step sign-in, which is off here.
    let leftover = serde_json::json!({
        "admin_password": pw, "code": "123456", "destination": tb
    });
    let r = post("/admin/exit/leftover", leftover.clone())
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(text(r).await.contains("two-step"));
    // Turning it off needs the password; then all of the fee
    // money can go back (again, as far as the two-step check here).
    let r = post(
        "/admin/exit/optin",
        serde_json::json!({"on": false, "admin_password": "nope nope nope"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert_eq!(get().await["exit_opted_in"], true);
    let r: serde_json::Value = post(
        "/admin/exit/optin",
        serde_json::json!({"on": false, "admin_password": pw}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(r["opted_in"], false);
    let g = get().await;
    assert_eq!(g["exit_opted_in"], false);
    let r = post("/admin/exit/leftover", leftover).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(text(r).await.contains("two-step"));
}

/// The code an authenticator app would show for a stored secret at a 30-second step.
fn app_code(secret_b32: &str, step: i64) -> String {
    let key = easy_bitcoin_payments::totp::base32_decode(secret_b32).unwrap();
    easy_bitcoin_payments::totp::code_for_step(&key, step)
}

/// A software passkey: signs registrations and sign-ins the way a browser's passkey would.
struct SoftKey {
    pair: ring::signature::EcdsaKeyPair,
    cred_id: Vec<u8>,
    point: Vec<u8>,
}

impl SoftKey {
    fn new() -> Self {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let point = pair.public_key().as_ref().to_vec();
        let cred_id = point[1..33].to_vec();
        SoftKey {
            pair,
            cred_id,
            point,
        }
    }

    fn id(&self) -> String {
        easy_bitcoin_payments::passkey::b64url_encode(&self.cred_id)
    }

    /// What a browser sends back to `/admin/passkeys/finish` (without the challenge and name).
    fn registration(&self, challenge_b64: &str, rp_id: &str, origin: &str) -> serde_json::Value {
        use easy_bitcoin_payments::passkey::b64url_encode as b64;
        fn head(major: u8, n: usize) -> Vec<u8> {
            if n < 24 {
                vec![(major << 5) | (n as u8)]
            } else {
                vec![(major << 5) | 24, n as u8]
            }
        }
        fn text(s: &str) -> Vec<u8> {
            let mut v = head(3, s.len());
            v.extend_from_slice(s.as_bytes());
            v
        }
        let point = &self.point;
        let mut cose = vec![0xA5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        cose.extend_from_slice(&point[1..33]);
        cose.extend_from_slice(&[0x22, 0x58, 0x20]);
        cose.extend_from_slice(&point[33..65]);
        let mut auth = ring::digest::digest(&ring::digest::SHA256, rp_id.as_bytes())
            .as_ref()
            .to_vec();
        auth.push(0x01 | 0x40 | 0x08 | 0x10); // present, new passkey, synced
        auth.extend_from_slice(&[0, 0, 0, 0]);
        auth.extend_from_slice(&[0u8; 16]);
        auth.extend_from_slice(&(self.cred_id.len() as u16).to_be_bytes());
        auth.extend_from_slice(&self.cred_id);
        auth.extend_from_slice(&cose);
        let mut att = vec![0xA3];
        att.extend(text("fmt"));
        att.extend(text("none"));
        att.extend(text("attStmt"));
        att.push(0xA0);
        att.extend(text("authData"));
        att.extend(head(2, auth.len()));
        att.extend_from_slice(&auth);
        let client = format!(
            r#"{{"type":"webauthn.create","challenge":"{challenge_b64}","origin":"{origin}","crossOrigin":false}}"#
        );
        serde_json::json!({
            "id": b64(&self.cred_id),
            "client_data_json": b64(client.as_bytes()),
            "attestation_object": b64(&att),
        })
    }

    /// What a browser sends back after `navigator.credentials.get` (a synced passkey:
    /// counter 0), with the challenge included.
    fn assertion(&self, challenge_b64: &str, rp_id: &str, origin: &str) -> serde_json::Value {
        use easy_bitcoin_payments::passkey::b64url_encode as b64;
        let rng = ring::rand::SystemRandom::new();
        let mut auth = ring::digest::digest(&ring::digest::SHA256, rp_id.as_bytes())
            .as_ref()
            .to_vec();
        auth.push(0x01 | 0x08 | 0x10); // present, synced
        auth.extend_from_slice(&[0, 0, 0, 0]);
        let client = format!(
            r#"{{"type":"webauthn.get","challenge":"{challenge_b64}","origin":"{origin}","crossOrigin":false}}"#
        );
        let mut signed = auth.clone();
        signed.extend_from_slice(
            ring::digest::digest(&ring::digest::SHA256, client.as_bytes()).as_ref(),
        );
        let sig = self.pair.sign(&rng, &signed).unwrap();
        serde_json::json!({
            "challenge": challenge_b64,
            "id": b64(&self.cred_id),
            "client_data_json": b64(client.as_bytes()),
            "authenticator_data": b64(&auth),
            "signature": b64(sig.as_ref()),
        })
    }
}

fn soft_registration(challenge_b64: &str, rp_id: &str, origin: &str) -> serde_json::Value {
    SoftKey::new().registration(challenge_b64, rp_id, origin)
}

#[tokio::test]
async fn passkeys_need_the_app_code_first_are_added_and_removed_and_hold_the_app_code_on() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let step0 = now_unix() / 30;
    let config = Config {
        password_hash_iterations: 1_000,
        trust_proxy_headers: true,
        public_base_url: "https://pay.example.com".to_string(),
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let pw = "a good long password";
    state
        .db
        .call(move |c| easy_bitcoin_payments::admin_auth::set_password(c, pw, 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let n = AtomicU32::new(1);
    let send = |path: &str, session: Option<(&str, &str)>, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header(
                "x-forwarded-for",
                format!("10.4.0.{}", n.fetch_add(1, Ordering::SeqCst)),
            )
            .json(&body);
        if let Some((cookie, csrf)) = session {
            r = r.header("cookie", cookie).header("x-ebp-csrf", csrf);
        }
        r.send()
    };
    let login = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    assert_eq!(login.status().as_u16(), 200);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let sess = Some((cookie.as_str(), csrf.as_str()));
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };

    // Not signed in: refused.
    let r = send("/admin/passkeys/status", None, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // The app code is off: passkeys cannot be added yet, and the page says why.
    let st = json(
        send("/admin/passkeys/status", sess, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st["two_step_on"], false);
    assert_eq!(st["unavailable"], serde_json::Value::Null);
    let r = send(
        "/admin/passkeys/begin",
        sess,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(json(r).await.to_string().contains("app code first"));

    // Turn the app code on.
    let started = json(
        send(
            "/admin/2fa/start",
            sess,
            serde_json::json!({"admin_password": pw}),
        )
        .await
        .unwrap(),
    )
    .await;
    let secret = started["secret"].as_str().unwrap().to_string();
    let done = json(
        send(
            "/admin/2fa/confirm",
            sess,
            serde_json::json!({"code": app_code(&secret, step0)}),
        )
        .await
        .unwrap(),
    )
    .await;
    let codes: Vec<String> = done["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();

    // Adding one asks for the password and a code.
    let r = send(
        "/admin/passkeys/begin",
        sess,
        serde_json::json!({"admin_password": "nope nope nope nope", "code": codes[0]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send(
        "/admin/passkeys/begin",
        sess,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "no code, no passkey");

    // An answer made for another web address is refused and nothing is saved.
    let begun = json(
        send(
            "/admin/passkeys/begin",
            sess,
            serde_json::json!({"admin_password": pw, "code": codes[0]}),
        )
        .await
        .unwrap(),
    )
    .await;
    let opts = &begun["options"];
    assert_eq!(opts["rp"]["id"], "pay.example.com");
    assert_eq!(opts["attestation"], "none");
    let challenge = opts["challenge"].as_str().unwrap().to_string();
    let mut wrong = soft_registration(&challenge, "pay.example.com", "https://evil.example.net");
    wrong["challenge"] = serde_json::json!(challenge);
    let r = send("/admin/passkeys/finish", sess, wrong).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let st = json(
        send("/admin/passkeys/status", sess, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st["passkeys"].as_array().unwrap().len(), 0);

    // A good one is saved; the same request cannot be used twice.
    let begun = json(
        send(
            "/admin/passkeys/begin",
            sess,
            serde_json::json!({"admin_password": pw, "code": codes[1]}),
        )
        .await
        .unwrap(),
    )
    .await;
    let challenge = begun["options"]["challenge"].as_str().unwrap().to_string();
    let mut good = soft_registration(&challenge, "pay.example.com", "https://pay.example.com");
    good["challenge"] = serde_json::json!(challenge);
    good["name"] = serde_json::json!("  Bitwarden  on my laptop ");
    let r = send("/admin/passkeys/finish", sess, good.clone())
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let saved = json(r).await;
    assert_eq!(saved["passkeys"].as_array().unwrap().len(), 1);
    assert_eq!(saved["passkeys"][0]["name"], "Bitwarden on my laptop");
    assert_eq!(saved["passkeys"][0]["synced"], true);
    let r = send("/admin/passkeys/finish", sess, good).await.unwrap();
    assert_eq!(r.status().as_u16(), 400, "a challenge works once");
    let first_id = saved["passkeys"][0]["id"].as_i64().unwrap();
    let st = json(
        send("/admin/2fa/status", sess, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st["passkeys"], 1);

    // While passkeys exist the app code cannot be turned off, and the refusal
    // does not use up a code.
    let r = send(
        "/admin/2fa/disable",
        sess,
        serde_json::json!({"admin_password": pw, "code": codes[2]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(json(r)
        .await
        .to_string()
        .contains("Turn off your passkeys first"));

    // Removing one passkey needs the password and a code; an unknown one is refused.
    let r = send(
        "/admin/passkeys/remove",
        sess,
        serde_json::json!({"admin_password": pw, "code": codes[2], "id": first_id + 100}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send(
        "/admin/passkeys/remove",
        sess,
        serde_json::json!({"admin_password": pw, "id": first_id}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "no code, no removal");

    // Turn all passkeys off, then the app code can be turned off.
    let r = send(
        "/admin/passkeys/disable",
        sess,
        serde_json::json!({"admin_password": pw, "code": codes[3]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(json(r).await["passkeys"].as_array().unwrap().len(), 0);
    let r = send(
        "/admin/2fa/disable",
        sess,
        serde_json::json!({"admin_password": pw, "code": codes[4]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

#[tokio::test]
async fn two_step_sign_in_turns_on_gates_sign_in_and_actions_and_turns_off() {
    use std::sync::atomic::{AtomicU32, Ordering};
    // Fixed once, so a 30-second boundary passing mid-test cannot change the codes used.
    let step0 = now_unix() / 30;
    let config = Config {
        password_hash_iterations: 1_000,
        // Each request below pretends to come from its own address so the
        // password-guessing brake (a few tries a minute per client) does not interfere.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let pw = "a good long password";
    state
        .db
        .call(move |c| easy_bitcoin_payments::admin_auth::set_password(c, pw, 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let n = AtomicU32::new(1);
    let send = |path: &str, session: Option<(&str, &str)>, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header(
                "x-forwarded-for",
                format!("10.1.0.{}", n.fetch_add(1, Ordering::SeqCst)),
            )
            .json(&body);
        if let Some((cookie, csrf)) = session {
            r = r.header("cookie", cookie).header("x-ebp-csrf", csrf);
        }
        r.send()
    };
    let cookie_of = |r: &reqwest::Response| {
        r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    };

    // Sign in with the password only (two-step is off).
    let login = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    assert_eq!(login.status().as_u16(), 200);
    let cookie = cookie_of(&login);
    let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let sess = Some((cookie.as_str(), csrf.as_str()));

    // Turning it on needs the password; a wrong one is refused.
    let r = send(
        "/admin/2fa/start",
        sess,
        serde_json::json!({"admin_password": "nope nope nope nope"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let started: serde_json::Value = send(
        "/admin/2fa/start",
        sess,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    let secret = started["secret"].as_str().unwrap().to_string();
    assert!(started["uri"]
        .as_str()
        .unwrap()
        .starts_with("otpauth://totp/"));

    // Not on yet: sign-in still works without a code. A wrong code does not turn it on.
    let r = send(
        "/admin/2fa/confirm",
        sess,
        serde_json::json!({"code": "000000"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let done: serde_json::Value = send(
        "/admin/2fa/confirm",
        sess,
        serde_json::json!({"code": app_code(&secret, step0)}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(done["enabled"], true);
    let codes: Vec<String> = done["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert_eq!(codes.len(), 10);

    // The session now reports it.
    let s: serde_json::Value = client
        .get(format!("{base}/admin/session"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["twofa"], true);

    // Signing in: the password alone no longer gets a session.
    let r = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(r.headers().get("set-cookie").is_none());
    assert_eq!(
        r.json::<serde_json::Value>().await.unwrap()["code_required"],
        true
    );
    // A wrong code, and a wrong password with a right code, are refused.
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": "000000"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": "nope nope nope nope", "code": app_code(&secret, step0 + 1)}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // The code used to turn it on cannot be used again to sign in.
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": app_code(&secret, step0)}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // The next code works.
    let login = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": app_code(&secret, step0 + 1)}),
    )
    .await
    .unwrap();
    assert_eq!(login.status().as_u16(), 200);
    let cookie2 = cookie_of(&login);
    let csrf2 = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let sess2 = Some((cookie2.as_str(), csrf2.as_str()));

    // A recovery code works once.
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": codes[0]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(r.headers().get("set-cookie").is_some());
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": codes[0]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // Protected actions ask for the code too (the sign-in code can be reused right after).
    let a = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
    let r = send(
        "/admin/payout",
        sess2,
        serde_json::json!({"address": a, "admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send(
        "/admin/payout",
        sess2,
        serde_json::json!({"address": a, "admin_password": pw, "code": "000000"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send("/admin/payout", sess2, serde_json::json!({"address": a, "admin_password": pw, "code": app_code(&secret, step0 + 1)}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = send(
        "/admin/backup",
        sess2,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap();
    // Refused without a code (400). Another backup test running at the same
    // moment can make it a 409 "one at a time" instead; either way no file.
    assert!(
        [400, 409].contains(&r.status().as_u16()),
        "backup without a code must be refused, got {}",
        r.status()
    );

    // Turning it off needs the password and a code, then sign-in is password-only again.
    let r = send(
        "/admin/2fa/disable",
        sess2,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // The code that approved the payout change above is spent: it cannot approve
    // a second action, and that is not counted as a wrong code.
    let r = send(
        "/admin/2fa/disable",
        sess2,
        serde_json::json!({"admin_password": pw, "code": app_code(&secret, step0 + 1)}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "a code approves one action");
    // A backup code is a fresh one.
    let r = send(
        "/admin/2fa/disable",
        sess2,
        serde_json::json!({"admin_password": pw, "code": codes[1]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    assert!(r.headers().get("set-cookie").is_some());
}

#[tokio::test]
async fn five_wrong_codes_lock_two_step_for_ten_minutes_even_for_the_right_code() {
    use easy_bitcoin_payments::totp::{self, Second};
    let (_base, state, _id, _key) = serve().await;
    state
        .db
        .call(|c| {
            let secret = totp::begin_setup(c)?;
            let key = totp::base32_decode(&secret).unwrap();
            let now = now_unix();
            totp::confirm(c, &totp::code_for_step(&key, now / 30), now)?;
            // Wrong tries up to the limit (a different step than the one just used).
            for i in 0..totp::MAX_FAILS {
                let r = totp::verify(c, "000000", now, false)?;
                assert_eq!(
                    r,
                    if i < totp::MAX_FAILS - 1 {
                        Second::Wrong
                    } else {
                        Second::Locked
                    },
                    "try {i}"
                );
            }
            // Locked: even the right code is refused...
            let good = totp::code_for_step(&key, now / 30 + 1);
            assert_eq!(totp::verify(c, &good, now, false)?, Second::Locked);
            // ...until ten minutes pass.
            let later = now + 601;
            let good_later = totp::code_for_step(&key, later / 30);
            assert_eq!(totp::verify(c, &good_later, later, false)?, Second::Ok);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn pause_switch_refuses_new_payments_only_and_the_shop_key_cannot_undo_it() {
    let (base, state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let new_invoice = || {
        client
            .post(format!("{base}/api/v1/invoices"))
            .header("host", "localhost")
            .bearer_auth(&key)
            .json(&serde_json::json!({"sats": 1000, "order_id": format!("o-{}", easy_bitcoin_payments::util::random_hex(4))}))
            .send()
    };
    let pause = |paused: bool, session: bool| {
        let mut r = client
            .post(format!("{base}/admin/pause"))
            .header("host", "localhost")
            .json(&serde_json::json!({ "paused": paused }));
        if session {
            r = r.header("cookie", &cookie).header("x-ebp-csrf", &csrf);
        } else {
            r = r.bearer_auth(&key);
        }
        r.send()
    };

    let made: serde_json::Value = new_invoice().await.unwrap().json().await.unwrap();
    let existing = made["id"].as_str().unwrap().to_string();

    // The shop's key cannot pause or resume; the owner can.
    assert_eq!(pause(true, false).await.unwrap().status().as_u16(), 401);
    assert_eq!(pause(true, true).await.unwrap().status().as_u16(), 200);

    // New requests are refused with a plain message; existing ones still load.
    let refused = new_invoice().await.unwrap();
    assert_eq!(refused.status().as_u16(), 409);
    let body: serde_json::Value = refused.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("paused"));
    let still = client
        .get(format!("{base}/api/v1/invoices/{existing}"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(still.status().as_u16(), 200);

    // The session reports it, and resuming works.
    let s: serde_json::Value = client
        .get(format!("{base}/admin/session"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s["paused"], true);
    assert_eq!(pause(false, true).await.unwrap().status().as_u16(), 200);
    assert_eq!(new_invoice().await.unwrap().status().as_u16(), 201);
}

#[tokio::test]
async fn shop_key_can_be_locked_to_the_shops_address() {
    let config = Config {
        // Pretend requests come from the address in X-Forwarded-For.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, key) = serve_with(config).await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let from = |ip: &'static str| {
        client
            .get(format!("{base}/api/v1/invoices?limit=1"))
            .header("host", "localhost")
            .header("x-forwarded-for", ip)
            .bearer_auth(&key)
            .send()
    };
    let admin = |path: &str, body: serde_json::Value| {
        client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .header("x-ebp-csrf", &csrf)
            .json(&body)
            .send()
    };

    // No limit yet: anywhere works.
    assert_eq!(from("198.51.100.9").await.unwrap().status().as_u16(), 200);

    // A bad entry is refused in plain words; a good one is saved.
    let bad = admin(
        "/admin/keylock/save",
        serde_json::json!({"addresses": "my-shop.com"}),
    )
    .await
    .unwrap();
    assert_eq!(bad.status().as_u16(), 400);
    let saved: serde_json::Value = admin(
        "/admin/keylock/save",
        serde_json::json!({"addresses": "203.0.113.7"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(saved["addresses"][0], "203.0.113.7");

    // Now only the shop's address works; the refused one is reported to the owner.
    assert_eq!(from("203.0.113.7").await.unwrap().status().as_u16(), 200);
    assert_eq!(from("198.51.100.9").await.unwrap().status().as_u16(), 401);
    // A stranger with a made-up key must not leave a hint for the owner.
    let stranger = client
        .get(format!("{base}/api/v1/invoices?limit=1"))
        .header("host", "localhost")
        .header("x-forwarded-for", "192.0.2.55")
        .bearer_auth("ebp_not_a_real_key")
        .send()
        .await
        .unwrap();
    assert_eq!(stranger.status().as_u16(), 401);
    let status: serde_json::Value = admin("/admin/keylock/status", serde_json::json!({}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["last_blocked_address"], "198.51.100.9");

    // The owner (admin page) is never locked out, and can remove the limit.
    let removed = admin("/admin/keylock/save", serde_json::json!({"addresses": ""}))
        .await
        .unwrap();
    assert_eq!(removed.status().as_u16(), 200);
    assert_eq!(from("198.51.100.9").await.unwrap().status().as_u16(), 200);
}

#[tokio::test]
async fn payment_summary_setting_defaults_to_daily_and_can_be_changed() {
    let (base, state, _store_id, _key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let post = |path: &str, body: serde_json::Value| {
        client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .header("x-ebp-csrf", &csrf)
            .json(&body)
            .send()
    };
    let v: serde_json::Value = post("/admin/alerts/status", serde_json::json!({}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        v["state"], "undecided",
        "alerts stay off until the owner turns them on"
    );
    assert_eq!(v["activity"], "daily");
    let v: serde_json::Value = post(
        "/admin/alerts/activity",
        serde_json::json!({"mode": "each"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(v["activity"], "each");
    let bad = post(
        "/admin/alerts/activity",
        serde_json::json!({"mode": "hourly"}),
    )
    .await
    .unwrap();
    assert_eq!(bad.status().as_u16(), 400);
}

#[tokio::test]
async fn the_second_step_guards_resume_the_address_lock_and_alerts_but_never_pause_and_only_when_on(
) {
    use easy_bitcoin_payments::totp;
    let (base, state, _store_id, _key) = serve().await;
    let client = reqwest::Client::new();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let post = |path: &str, body: serde_json::Value| {
        client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .header("x-ebp-csrf", &csrf)
            .json(&body)
            .send()
    };
    let code_of = |r: reqwest::Response| r.status().as_u16();

    // Two-step sign-in is OFF: none of these ever asks for a code.
    for (path, body) in [
        ("/admin/pause", serde_json::json!({"paused": true})),
        ("/admin/pause", serde_json::json!({"paused": false})),
        ("/admin/keylock/save", serde_json::json!({"addresses": ""})),
        ("/admin/alerts/enable", serde_json::json!({})),
        ("/admin/alerts/disable", serde_json::json!({})),
    ] {
        assert_eq!(code_of(post(path, body).await.unwrap()), 200, "{path}");
    }

    // Turn it on directly (as the setup flow would).
    let step0 = now_unix() / 30;
    let secret = state.db.call(|c| totp::begin_setup(c)).await.unwrap();
    let key = totp::base32_decode(&secret).unwrap();
    let first = totp::code_for_step(&key, step0);
    let codes = state
        .db
        .call(move |c| totp::confirm(c, &first, step0 * 30 + 1))
        .await
        .unwrap();

    // Pausing is the emergency button: no code, ever.
    assert_eq!(
        code_of(
            post("/admin/pause", serde_json::json!({"paused": true}))
                .await
                .unwrap()
        ),
        200
    );
    // Resuming asks for one.
    assert_eq!(
        code_of(
            post("/admin/pause", serde_json::json!({"paused": false}))
                .await
                .unwrap()
        ),
        400
    );
    let resume = |code: String| {
        post(
            "/admin/pause",
            serde_json::json!({"paused": false, "code": code}),
        )
    };
    assert_eq!(
        code_of(resume(totp::code_for_step(&key, step0)).await.unwrap()),
        200
    );
    // The same code does not approve a second action, and the message says what to do.
    assert_eq!(
        code_of(
            post("/admin/pause", serde_json::json!({"paused": true}))
                .await
                .unwrap()
        ),
        200
    );
    let again = resume(totp::code_for_step(&key, step0)).await.unwrap();
    assert_eq!(again.status().as_u16(), 400);
    let body: serde_json::Value = again.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already used"),
        "{body}"
    );
    // A backup code is a fresh one.
    assert_eq!(code_of(resume(codes[0].clone()).await.unwrap()), 200);

    // The shop-address lock.
    let lock = |code: Option<String>| {
        let mut b = serde_json::json!({"addresses": "203.0.113.7"});
        if let Some(c) = code {
            b["code"] = serde_json::Value::from(c);
        }
        post("/admin/keylock/save", b)
    };
    assert_eq!(code_of(lock(None).await.unwrap()), 400);
    assert_eq!(code_of(lock(Some("000000".into())).await.unwrap()), 400);
    assert_eq!(code_of(lock(Some(codes[1].clone())).await.unwrap()), 200);

    // Alerts: turning them on with the default server needs nothing; an own
    // server and turning working alerts off ask for a code.
    assert_eq!(
        code_of(
            post("/admin/alerts/enable", serde_json::json!({}))
                .await
                .unwrap()
        ),
        200
    );
    assert_eq!(
        code_of(
            post(
                "/admin/alerts/enable",
                serde_json::json!({"server": "https://ntfy.example.com"})
            )
            .await
            .unwrap()
        ),
        400
    );
    assert_eq!(
        code_of(
            post("/admin/alerts/disable", serde_json::json!({}))
                .await
                .unwrap()
        ),
        400,
        "turning working alerts off asks for a code"
    );
    assert_eq!(
        code_of(
            post(
                "/admin/alerts/disable",
                serde_json::json!({"code": codes[2]})
            )
            .await
            .unwrap()
        ),
        200
    );
    // Already off: nothing to guard.
    assert_eq!(
        code_of(
            post("/admin/alerts/disable", serde_json::json!({}))
                .await
                .unwrap()
        ),
        200
    );
}

#[tokio::test]
async fn password_change_checks_the_current_password_and_the_code_and_signs_everyone_out() {
    use easy_bitcoin_payments::totp;
    let step0 = now_unix() / 30;
    let config = Config {
        password_hash_iterations: 1_000,
        // Each call below says which address it comes from, so the
        // password-guessing brake (a few tries a minute per client) is easy to steer.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let pw = "a good long password";
    let new_pw = "another good long password";
    state
        .db
        .call(move |c| easy_bitcoin_payments::admin_auth::set_password(c, pw, 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let send = |path: &str, ip: &str, session: Option<(&str, &str)>, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("x-forwarded-for", ip.to_string())
            .json(&body);
        if let Some((cookie, csrf)) = session {
            r = r
                .header("cookie", cookie.to_string())
                .header("x-ebp-csrf", csrf.to_string());
        }
        r.send()
    };

    let login = send(
        "/admin/login",
        "10.2.0.1",
        None,
        serde_json::json!({"password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(login.status().as_u16(), 200);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let csrf = login.json::<serde_json::Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let sess = Some((cookie.as_str(), csrf.as_str()));

    // A mistyped current password is a plain error (400), not "signed out" (401).
    let r = send(
        "/admin/password",
        "10.2.0.2",
        sess,
        serde_json::json!({"current": "not the password at all", "new": new_pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // A too-short new password is refused.
    let r = send(
        "/admin/password",
        "10.2.0.2",
        sess,
        serde_json::json!({"current": pw, "new": "short"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // With two-step on, the right password alone is not enough.
    let secret = state.db.call(|c| totp::begin_setup(c)).await.unwrap();
    let key = totp::base32_decode(&secret).unwrap();
    let code = totp::code_for_step(&key, step0);
    state
        .db
        .call(move |c| totp::confirm(c, &code, step0 * 30 + 1).map(|_| ()))
        .await
        .unwrap();
    let r = send(
        "/admin/password",
        "10.2.0.3",
        sess,
        serde_json::json!({"current": pw, "new": new_pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "no code");
    let r = send(
        "/admin/password",
        "10.2.0.3",
        sess,
        serde_json::json!({"current": pw, "new": new_pw, "code": "000000"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "wrong code");
    // Nothing changed: the old password still signs in (and asks for the code).
    let r = send(
        "/admin/login",
        "10.2.0.4",
        None,
        serde_json::json!({"password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(
        r.json::<serde_json::Value>().await.unwrap()["code_required"],
        true
    );

    // Right password and right code: changed, and every session is signed out.
    let r = send(
        "/admin/password",
        "10.2.0.5",
        sess,
        serde_json::json!({"current": pw, "new": new_pw, "code": totp::code_for_step(&key, step0)}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = send(
        "/admin/keylock/status",
        "10.2.0.6",
        sess,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401, "old session is gone");
    let r = send(
        "/admin/login",
        "10.2.0.7",
        None,
        serde_json::json!({"password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401, "old password no longer works");
    let r = send(
        "/admin/login",
        "10.2.0.8",
        None,
        serde_json::json!({"password": new_pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200, "new password works");

    // The route shares the password-guessing brake (a few tries a minute per client).
    let mut last = 0;
    for _ in 0..12 {
        last = send(
            "/admin/password",
            "10.2.0.99",
            None,
            serde_json::json!({"current": "x", "new": new_pw}),
        )
        .await
        .unwrap()
        .status()
        .as_u16();
    }
    assert_eq!(last, 429);
}

/// Garbage in must never produce a server error (5xx), a hang or a crash, on any
/// endpoint, with or without a signed-in admin. A 4xx (or 429) is the right answer.
#[tokio::test]
async fn malformed_requests_never_cause_server_errors() {
    let config = Config {
        password_hash_iterations: 1_000,
        // A fresh pretend address per request, so the sign-in brake does not
        // turn the answers into 429s and hide what the handlers do.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, key) = serve_with(config).await;
    let (cookie, csrf) = owner_session(&base, &state).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap();

    let big = format!("{{\"order_id\":\"{}\",\"sats\":1}}", "a".repeat(2_000_000));
    let wrong_types = r#"{"admin_password":123,"password":[],"current":{},"new":5,"code":[],"address":{},"addresses":5,"paused":"yes","order_id":null,"sats":"x","fiat":5,"metadata":"x","url":7,"mode":9,"answers":"x","name":[],"token":{},"confirmed":"x","rotate_secret":"x","resolution":3,"note":[]}"#;
    let bodies: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("broken json", b"{".to_vec()),
        ("null", b"null".to_vec()),
        ("array", b"[]".to_vec()),
        ("string", b"\"str\"".to_vec()),
        ("unknown field", br#"{"a":1}"#.to_vec()),
        ("wrong types", wrong_types.as_bytes().to_vec()),
        ("not utf-8", vec![0xff, 0xfe, 0x00, 0x80]),
        (
            "nul bytes",
            b"{\"order_id\":\"a\\u0000b\",\"sats\":1}".to_vec(),
        ),
        ("2 MB value", big.into_bytes()),
        (
            "deep nesting",
            format!("{}1{}", "[".repeat(5000), "]".repeat(5000)).into_bytes(),
        ),
    ];
    let posts = [
        "/admin/payout/cancel",
        "/admin/sweep",
        "/admin/alerts/status",
        "/admin/alerts/enable",
        "/admin/alerts/disable",
        "/admin/alerts/activity",
        "/admin/reconcile/status",
        "/admin/reconcile/done",
        "/admin/2fa/status",
        "/admin/2fa/skip",
        "/admin/pause",
        "/admin/keylock/status",
        "/admin/keylock/save",
        "/setup/wallet",
        "/setup/restore-backup",
        "/setup/wallet/confirm-backup",
        "/setup/wallet/mnemonic",
        "/setup/wallet/verify-backup",
        "/setup/verify-code",
        "/setup/token",
        "/setup/store",
        "/api/v1/invoices",
        "/api/v1/invoices/inv_x/cancel",
        "/api/v1/stores/webhook/test",
        "/api/v1/unmatched-payments/unm_x/resolve",
        "/api/v1/webhook-deliveries/whd_x/resend",
    ];

    // These share the sign-in brake: an overall cap of 20 a minute for requests
    // that are not signed in (no pretend address gets around it; past it they
    // answer 429, never 5xx). Signed-in requests are not counted.
    let braked = [
        "/admin/login",
        "/admin/payout",
        "/admin/password",
        "/admin/backup",
        "/admin/2fa/start",
        "/admin/2fa/confirm",
        "/admin/2fa/disable",
    ];
    let mut n: u32 = 0;
    let mut problems: Vec<String> = Vec::new();
    for path in braked {
        for (what, body) in &bodies {
            // (sign-in is the only one of these that makes sense without a session)
            for signed_in in [false, true] {
                if !signed_in && path != "/admin/login" {
                    continue;
                }
                n += 1;
                let mut r = client
                    .post(format!("{base}{path}"))
                    .header("host", "localhost")
                    .header("content-type", "application/json")
                    .header(
                        "x-forwarded-for",
                        format!("10.100.{}.{}", (n >> 8) & 255, n & 255),
                    )
                    .body(body.clone());
                if signed_in {
                    r = r.header("cookie", &cookie).header("x-ebp-csrf", &csrf);
                }
                match r.send().await {
                    Ok(resp) if resp.status().as_u16() >= 500 => problems.push(format!(
                        "POST {path} ({what}, signed_in={signed_in}) -> {}",
                        resp.status()
                    )),
                    Ok(_) => {}
                    Err(e) => problems.push(format!("POST {path} ({what}) -> no answer: {e}")),
                }
            }
        }
    }
    for signed_in in [false, true] {
        for path in posts {
            for (what, body) in &bodies {
                for ctype in [Some("application/json"), None] {
                    n += 1;
                    let mut r = client
                        .post(format!("{base}{path}"))
                        .header("host", "localhost")
                        .header(
                            "x-forwarded-for",
                            format!("10.{}.{}.{}", (n >> 16) & 255, (n >> 8) & 255, n & 255),
                        )
                        .body(body.clone());
                    if let Some(c) = ctype {
                        r = r.header("content-type", c);
                    }
                    r = if signed_in {
                        r.header("cookie", &cookie).header("x-ebp-csrf", &csrf)
                    } else {
                        r.bearer_auth(&key)
                    };
                    match r.send().await {
                        Ok(resp) if resp.status().as_u16() >= 500 => problems.push(format!(
                            "POST {path} ({what}, signed_in={signed_in}, {ctype:?}) -> {}",
                            resp.status()
                        )),
                        Ok(_) => {}
                        Err(e) => problems.push(format!("POST {path} ({what}) -> no answer: {e}")),
                    }
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} server errors, first ones:\n{}",
        problems.len(),
        problems
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Odd paths and query strings on the read side.
    let long = "x".repeat(5000);
    let gets = [
        "/api/v1/invoices?limit=abc".to_string(),
        "/api/v1/invoices?limit=-1&offset=-5".to_string(),
        "/api/v1/invoices?offset=99999999999999999999".to_string(),
        "/api/v1/invoices?status=%00%ff".to_string(),
        "/api/v1/invoices/%00".to_string(),
        format!("/api/v1/invoices/{long}"),
        "/api/v1/unmatched-payments?limit=0&offset=zzz".to_string(),
        "/api/v1/webhook-deliveries?status=%27%3B--&limit=1000000".to_string(),
        "/api/v1/audit-log?limit=-3".to_string(),
        "/i/%00".to_string(),
        format!("/i/{long}"),
        format!("/i/{long}/json"),
        format!("/i/{long}/status"),
        format!("/i/{long}/rails"),
        "/i/..%2f..%2fetc%2fpasswd".to_string(),
        "/assets/../Cargo.toml".to_string(),
        "/setup/status".to_string(),
        "/admin/session".to_string(),
    ];
    for signed_in in [false, true] {
        for path in &gets {
            n += 1;
            let mut r = client
                .get(format!("{base}{path}"))
                .header("host", "localhost")
                .header(
                    "x-forwarded-for",
                    format!("10.200.{}.{}", (n >> 8) & 255, n & 255),
                );
            r = if signed_in {
                r.header("cookie", &cookie)
            } else {
                r.bearer_auth(&key)
            };
            match r.send().await {
                Ok(resp) if resp.status().as_u16() >= 500 => problems.push(format!(
                    "GET {path} (signed_in={signed_in}) -> {}",
                    resp.status()
                )),
                Ok(_) => {}
                Err(e) => problems.push(format!("GET {path} -> no answer: {e}")),
            }
        }
    }
    assert!(
        problems.is_empty(),
        "{} server errors, first ones:\n{}",
        problems.len(),
        problems
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Still healthy, and the shop key still works for a normal invoice.
    let health = client
        .get(format!("{base}/api/v1/health"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(health.status().as_u16(), 200);
    let (code, _) = create(&client, &base, &key, "after-garbage", 1500).await;
    assert!(
        code == 200 || code == 201,
        "normal invoice after garbage: {code}"
    );
}

#[tokio::test]
async fn the_dollar_price_reaches_the_checkout_page_and_the_admin_status_only_when_known() {
    let client = reqwest::Client::new();

    // Known price: both carry it.
    let (base, _s, _store, key) = serve_with_rates(
        Config {
            admin_token: Some("adm1n".to_string()),
            ..Config::default()
        },
        RateService::seeded_for_tests(&[("USD", 40_000.0)]),
    )
    .await;
    let (_, inv) = create(&client, &base, &key, "wc-usd", 1_000).await;
    let id = inv["id"].as_str().unwrap();
    let co: serde_json::Value = client
        .get(format!("{base}/i/{id}/json"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(co["usd_per_btc"], 40_000.0);
    let st: serde_json::Value = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth("adm1n")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(st["usd_per_btc"], 40_000.0);
    assert!(st["price_age_secs"].as_i64().unwrap() < 60);
    assert!(
        st["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != "price"),
        "a fresh price needs no warning"
    );

    // No price yet (the server just started, or no source has ever answered):
    // null, a plain warning on Home, and sats-priced payments are unaffected.
    let (base, _s, _store, key) = serve_with(Config {
        admin_token: Some("adm1n".to_string()),
        ..Config::default()
    })
    .await;
    let (status, inv) = create(&client, &base, &key, "wc-nousd", 1_000).await;
    assert_eq!(status, 201);
    let id = inv["id"].as_str().unwrap();
    let co: serde_json::Value = client
        .get(format!("{base}/i/{id}/json"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(co["usd_per_btc"].is_null());
    assert_eq!(co["amount_sats"], 1_000);
    let st: serde_json::Value = client
        .get(format!("{base}/api/v1/admin/status"))
        .header("host", "localhost")
        .bearer_auth("adm1n")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(st["usd_per_btc"].is_null());
    assert!(st["price_age_secs"].is_null());
    assert!(st["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == "price" && c["level"] == "warn"));
}

#[tokio::test]
async fn an_order_priced_in_a_currency_uses_the_stored_price_and_is_refused_without_one() {
    let client = reqwest::Client::new();
    let post = |base: String, key: String| {
        let client = client.clone();
        async move {
            let resp = client
                .post(format!("{base}/api/v1/invoices"))
                .header("host", "localhost")
                .bearer_auth(key)
                .json(&serde_json::json!({
                    "order_id": "eur-1",
                    "fiat": { "amount": "10.00", "currency": "eur" }
                }))
                .send()
                .await
                .unwrap();
            let status = resp.status().as_u16();
            (status, resp.json::<serde_json::Value>().await.unwrap())
        }
    };

    // With a price: 10 EUR at 100,000 EUR/BTC is exactly 10,000 sats.
    let (base, _s, _store, key) = serve_with_rates(
        Config::default(),
        RateService::seeded_for_tests(&[("EUR", 100_000.0)]),
    )
    .await;
    let (status, inv) = post(base, key).await;
    assert_eq!(status, 201, "{inv}");
    assert_eq!(inv["amount_sats"], 10_000);

    // Without one: a clear, temporary refusal (not a crash, not a guess).
    let (base, _s, _store, key) = serve().await;
    let (status, body) = post(base, key).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "unavailable");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not available right now"));
}

/// The wizard's "bring back my backup file" step: it only works on an
/// unfinished server whose wallet was restored from the 12 words, and a bad
/// file or password changes nothing. (The success path restarts the server,
/// so it is covered by the unit tests of `backup::stage_restore` and
/// `backup::apply_pending_restore` instead.)
#[tokio::test]
async fn wizard_backup_restore_needs_the_words_first_and_refuses_bad_files() {
    use easy_bitcoin_payments::backup::{seal_with, Entry};

    let dir = std::env::temp_dir().join(format!(
        "ebp-api-restore-{}",
        easy_bitcoin_payments::util::random_hex(6)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let config = Config {
        database_path: dir.join("ebp.sqlite3").to_string_lossy().to_string(),
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let client = reqwest::Client::new();
    let pw = "a good long password";
    let body_for = |password: &str, file: &[u8]| {
        let mut b = (password.len() as u32).to_be_bytes().to_vec();
        b.extend_from_slice(password.as_bytes());
        b.extend_from_slice(file);
        b
    };
    let post = |body: Vec<u8>| {
        client
            .post(format!("{base}/setup/restore-backup"))
            .header("host", "localhost")
            .header("content-type", "application/octet-stream")
            .body(body)
            .send()
    };
    let junk_backup = seal_with(
        &[
            Entry {
                name: "ebp.sqlite3".into(),
                data: b"this is not a database".to_vec(),
            },
            Entry {
                name: "ebp.key".into(),
                data: "a".repeat(64).into_bytes(),
            },
        ],
        pw,
        1_000,
    )
    .unwrap();

    // 1. A finished server never takes a restore.
    let r = post(body_for(pw, &junk_backup)).await.unwrap();
    assert_eq!(r.status().as_u16(), 409);

    // 2. Unfinished, but the wallet was not restored from the words: refused.
    state
        .db
        .call(|c| easy_bitcoin_payments::settings::set(c, "setup_complete", "false"))
        .await
        .unwrap();
    let r = post(body_for(pw, &junk_backup)).await.unwrap();
    assert_eq!(r.status().as_u16(), 409);
    let text = r.text().await.unwrap();
    assert!(text.contains("12 recovery words"), "{text}");

    // 3. Wallet restored from the words: a wrong password and a file that is
    //    not a backup are refused, and nothing is staged.
    state
        .db
        .call(|c| easy_bitcoin_payments::settings::set(c, "setup_wallet_restored", "true"))
        .await
        .unwrap();
    let r = post(body_for("not the password at all", &junk_backup))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // Bigger than the 64 KB limit other requests have: the upload must still be
    // read (real backups are megabytes) and refused only for what it is.
    let r = post(body_for(pw, &vec![b'x'; 200 * 1024])).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let text = r.text().await.unwrap();
    assert!(
        text.contains("not an Easy Bitcoin Payments backup"),
        "{text}"
    );
    let r = post(body_for(pw, &junk_backup)).await.unwrap();
    assert_eq!(r.status().as_u16(), 400, "a backup with no usable database");
    assert!(!dir.join("restore-pending").exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn emergency_withdrawal_needs_a_session_the_tick_box_an_onchain_address_and_two_step() {
    let config = Config {
        password_hash_iterations: 1_000,
        // Its own address per request, so the password brake stays out of the way.
        trust_proxy_headers: true,
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let (cookie, csrf) = owner_session(&base, &state).await;
    let client = reqwest::Client::new();
    let n = std::sync::atomic::AtomicU32::new(1);
    let send = |path: &str, signed_in: bool, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header(
                "x-forwarded-for",
                format!(
                    "10.7.0.{}",
                    n.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                ),
            )
            .json(&body);
        if signed_in {
            r = r
                .header("cookie", cookie.clone())
                .header("x-ebp-csrf", csrf.clone());
        }
        r.send()
    };
    let tb = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
    let good = serde_json::json!({
        "admin_password": "a good long password", "code": "123456",
        "destination": tb, "understood": true
    });
    let message = |r: reqwest::Response| async move {
        let v: serde_json::Value = r.json().await.unwrap();
        v.to_string()
    };

    // Not signed in: refused, for every action.
    for path in [
        "/admin/exit/start",
        "/admin/exit/cancel",
        "/admin/exit/claim",
        "/admin/exit/leftover",
        "/admin/exit/status",
        "/admin/exit/deposit-address",
    ] {
        let r = send(path, false, good.clone()).await.unwrap();
        assert_eq!(r.status().as_u16(), 401, "{path}");
    }
    // The tick-box is required.
    let mut body = good.clone();
    body["understood"] = serde_json::json!(false);
    let r = send("/admin/exit/start", true, body).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(message(r).await.contains("Tick the box"));
    // Never to an Ark address, never to the wrong network.
    let mut body = good.clone();
    body["destination"] = serde_json::json!(
        "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"
    );
    let r = send("/admin/exit/start", true, body).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(message(r).await.contains("not an Ark address"));
    let mut body = good.clone();
    body["destination"] = serde_json::json!("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4");
    let r = send("/admin/exit/start", true, body).await.unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // Two-step sign-in is off: refused before anything touches the wallet,
    // for starting, stopping, claiming and the leftover.
    for path in [
        "/admin/exit/start",
        "/admin/exit/cancel",
        "/admin/exit/claim",
        "/admin/exit/leftover",
    ] {
        let r = send(path, true, good.clone()).await.unwrap();
        assert_eq!(r.status().as_u16(), 400, "{path}");
        assert!(message(r).await.contains("two-step"), "{path}");
    }
    // Nothing was started.
    let started = state
        .db
        .call(|c| {
            easy_bitcoin_payments::settings::get(
                c,
                easy_bitcoin_payments::emergency_exit::EXIT_STARTED_AT_KEY,
            )
        })
        .await
        .unwrap();
    assert!(started.is_none());
}

#[test]
fn the_settings_page_folds_the_emergency_withdrawal_away_behind_a_warning() {
    let page = include_str!("../assets/dashboard.html");
    let i = page
        .find("<details class=\"fold\" id=\"ex-box\">")
        .expect("folded away");
    let rest = &page[i..];
    assert!(
        rest.contains("Emergency: withdraw without the Ark server"),
        "titled in its folded row"
    );
    assert!(rest.contains("For emergencies only."));
    assert!(rest.contains("Move my money now"));
    assert!(
        rest.contains("id=\"ex-start\" type=\"button\" disabled"),
        "starts disabled"
    );
    // Only on the Settings tab, never on Home.
    let settings_at = page.find("<section id=\"v-set\"").unwrap();
    assert!(i > settings_at, "inside Settings");
    let home = &page[page.find("<section id=\"v-home\"").unwrap()
        ..page.find("<section id=\"v-orders\"").unwrap()];
    assert!(!home.contains("ex-"), "not on Home");
}

/// Text that looks like SQL, a script or a spreadsheet formula must be kept
/// as plain text: stored and read back exactly, never run, never matching
/// other rows, and the tables must survive it.
#[tokio::test]
async fn hostile_text_is_kept_as_plain_text_and_never_runs_as_code() {
    let (base, state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();

    let hostile = [
        "wc-1'; DROP TABLE invoices; --",
        "' OR '1'='1",
        "1); DELETE FROM stores; --",
        "x' || (SELECT api_key_hash FROM stores LIMIT 1) || '",
        "\" OR \"\"=\"",
        "100%",
        "a_b\\c",
        "<script>alert(1)</script>",
        "=HYPERLINK(\"http://example.invalid\",\"x\")",
        "wc-1",
    ];
    let mut made: Vec<(String, String, i64)> = Vec::new(); // (order, invoice id, sats)
    for (n, order) in hostile.iter().enumerate() {
        let sats = 1_000 + n as i64;
        let (status, body) = create(&client, &base, &key, order, sats).await;
        assert!(
            status == 201 || status == 400,
            "{order:?} must be accepted or refused, never a server error (got {status})"
        );
        if status == 201 {
            made.push((
                order.to_string(),
                body["id"].as_str().unwrap().to_string(),
                sats,
            ));
        }
    }
    assert!(made.len() >= 8, "ordinary-looking hostile text is accepted");

    // Every one reads back exactly as sent.
    for (order, id, _) in &made {
        let got = client
            .get(format!("{base}/api/v1/invoices/{id}"))
            .header("host", "localhost")
            .bearer_auth(&key)
            .send()
            .await
            .unwrap();
        assert_eq!(got.status(), 200);
        let json: serde_json::Value = got.json().await.unwrap();
        assert_eq!(json["order_id"].as_str(), Some(order.as_str()));
    }

    // The tables are all still there and nothing extra appeared or vanished.
    let (invoices, stores) = state
        .db
        .call(|c| {
            let i: i64 = c.query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))?;
            let s: i64 = c.query_row("SELECT COUNT(*) FROM stores", [], |r| r.get(0))?;
            Ok((i, s))
        })
        .await
        .unwrap();
    assert_eq!(invoices as usize, made.len());
    assert_eq!(stores, 1);

    // The same hostile order and amount is the same payment (idempotent) ...
    let (order, id, sats) = made
        .iter()
        .find(|(o, _, _)| o == "' OR '1'='1")
        .expect("the quote-and-OR text is accepted as an ordinary order id");
    let (again, body) = create(&client, &base, &key, order, *sats).await;
    assert_eq!(again, 200);
    assert_eq!(body["id"].as_str(), Some(id.as_str()));
    // ... but a quote or % in one order id never matches another order.
    let (plain_status, plain) = create(&client, &base, &key, "wc-2", *sats).await;
    assert_eq!(plain_status, 201, "a different order is a new payment");
    assert_ne!(plain["id"].as_str(), Some(id.as_str()));

    // Hostile text in a URL path or a key finds nothing and breaks nothing.
    for path in [
        "/api/v1/invoices/%27%20OR%20%271%27%3D%271",
        "/i/%27%20OR%20%271%27%3D%271/json",
        "/i/%27%3B%20DROP%20TABLE%20invoices%3B--/status",
    ] {
        let r = client
            .get(format!("{base}{path}"))
            .header("host", "localhost")
            .bearer_auth(&key)
            .send()
            .await
            .unwrap();
        assert!(
            r.status().as_u16() == 404 || r.status().as_u16() == 400,
            "{path} gave {}",
            r.status()
        );
    }
    let bad_key = client
        .get(format!("{base}/api/v1/invoices/{}", made[0].1))
        .header("host", "localhost")
        .bearer_auth("' OR '1'='1")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_key.status(), 401);

    // The customer's checkout data is JSON with a JSON content type and
    // nosniff, so script text in an order id is data, not markup; the
    // checkout page itself never echoes it.
    let script_invoice = made
        .iter()
        .find(|(o, _, _)| o.contains("<script>"))
        .expect("the script text is accepted as an ordinary order id");
    let json = client
        .get(format!("{base}/i/{}/json", script_invoice.1))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert!(json.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(json.headers()["x-content-type-options"], "nosniff");
    let page = client
        .get(format!("{base}/i/{}", script_invoice.1))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!page.contains("<script>alert(1)</script>"));

    // A spreadsheet formula in an order id is defused in the CSV export.
    let csv = client
        .get(format!("{base}/api/v1/invoices.csv"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        csv.contains("amount_sats,paid_sats,received_sats,fiat_currency"),
        "the export says what the wallet received"
    );
    assert!(
        csv.contains("'=HYPERLINK"),
        "formula is prefixed with a quote"
    );
    assert!(!csv.contains(",=HYPERLINK") && !csv.contains("\"=HYPERLINK"));
}

#[tokio::test]
async fn update_from_the_home_tab_needs_two_decisions_password_and_the_shown_digest() {
    // The helper's folders, in a throwaway place (only this test sets this).
    let dir = std::env::temp_dir().join(format!("ebp-updates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inbox")).unwrap();
    std::fs::create_dir_all(dir.join("status")).unwrap();
    std::env::set_var("EBP_UPDATES_DIR", &dir);
    let req_file = dir.join("inbox").join("request.json");
    let read_req = || -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(&req_file).unwrap()).unwrap()
    };

    let (base, state, _shop, _key) = serve_with(Config {
        update_check: false,
        password_hash_iterations: 1_000,
        trust_proxy_headers: true,
        ..Config::default()
    })
    .await;
    let (cookie, csrf) = owner_session(&base, &state).await;
    let client = reqwest::Client::new();
    // Each call comes from its own pretend address, so the password brake (a few tries
    // a minute per client) stays out of the way; the brake has its own step at the end.
    let calls = std::cell::Cell::new(0u32);
    let post_from = |from: String, path: &str, body: serde_json::Value, signed_in: bool| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header("x-forwarded-for", from)
            .json(&body);
        if signed_in {
            r = r.header("cookie", &cookie).header("x-ebp-csrf", &csrf);
        }
        r.send()
    };
    let post = |path: &str, body: serde_json::Value, signed_in: bool| {
        calls.set(calls.get() + 1);
        post_from(format!("10.9.0.{}", calls.get()), path, body, signed_in)
    };
    let newer = "99.0.0-rc1";
    let digest = format!("sha256:{}", "a".repeat(64));

    // Not signed in: refused.
    let r = post(
        "/admin/update/check",
        serde_json::json!({"version": newer}),
        false,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // Helper not installed yet: a clear refusal, and no request is written.
    let r = post(
        "/admin/update/check",
        serde_json::json!({"version": newer}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 409);
    assert!(r.text().await.unwrap().contains("not installed"));
    assert!(!req_file.exists());

    std::fs::write(
        dir.join("status").join("helper.json"),
        r#"{"helper_version":1,"at":1}"#,
    )
    .unwrap();

    // Never older, never the same, never something that is not a version.
    for (v, words) in [
        ("0.0.1", "older"),
        (env!("CARGO_PKG_VERSION"), "already runs"),
        ("1.0.0; reboot", "version number"),
    ] {
        let r = post(
            "/admin/update/check",
            serde_json::json!({"version": v}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(r.status().as_u16(), 400, "{v}");
        assert!(r.text().await.unwrap().contains(words), "{v}");
        assert!(!req_file.exists(), "no request for {v}");
    }

    // First decision: a check request with only the version and an id.
    let r = post(
        "/admin/update/check",
        serde_json::json!({"version": newer}),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let req = read_req();
    assert_eq!(req["action"], "check");
    assert_eq!(req["version"], newer);
    assert!(req.get("digest").is_none());
    std::fs::remove_file(&req_file).unwrap();

    // The version endpoint shows the helper and its answer.
    let status = format!(
        r#"{{"state":"ready","code":"ok","request_id":"{}","running":"{}","version":"{newer}","digest":"{digest}","commit":"{}","run_url":"","at":{}}}"#,
        req["id"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION"),
        "b".repeat(40),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    std::fs::write(dir.join("status").join("status.json"), status).unwrap();
    let v: serde_json::Value = client
        .get(format!("{base}/api/v1/admin/version"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["helper_version"], 1);
    assert_eq!(v["helper_status"]["state"], "ready");
    assert_eq!(v["helper_status"]["digest"], digest);

    // Second decision: a different digest than the one shown is refused.
    let apply =
        |d: &str, pw: &str| serde_json::json!({"version": newer, "digest": d, "password": pw});
    let other = format!("sha256:{}", "c".repeat(64));
    let r = post(
        "/admin/update/apply",
        apply(&other, "a good long password"),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 409);
    assert!(!req_file.exists());
    // A wrong password is refused.
    let r = post(
        "/admin/update/apply",
        apply(&digest, "not my password"),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(r.text().await.unwrap().contains("admin password"));
    assert!(!req_file.exists());
    // The right password and the shown digest: an apply request for exactly that digest.
    let r = post(
        "/admin/update/apply",
        apply(&digest, "a good long password"),
        true,
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let req = read_req();
    assert_eq!(req["action"], "apply");
    assert_eq!(req["version"], newer);
    assert_eq!(req["digest"], digest);

    // The update screens share the password brake: from one address, the tries past
    // the per-minute limit are refused (429) before any password is checked.
    std::fs::remove_file(&req_file).unwrap();
    let mut last = 0;
    for _ in 0..(easy_bitcoin_payments::ratelimit::PASSWORD_TRIES_PER_MINUTE + 1) {
        last = post_from(
            "10.9.9.9".to_string(),
            "/admin/update/apply",
            apply(&digest, "another guess"),
            true,
        )
        .await
        .unwrap()
        .status()
        .as_u16();
    }
    assert_eq!(last, 429);
    assert!(!req_file.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn passkeys_sign_in_approve_actions_and_backup_codes_still_work() {
    use std::sync::atomic::{AtomicU32, Ordering};
    const RP: &str = "pay.example.com";
    const ORIGIN: &str = "https://pay.example.com";
    let step0 = now_unix() / 30;
    let config = Config {
        password_hash_iterations: 1_000,
        trust_proxy_headers: true,
        public_base_url: ORIGIN.to_string(),
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    let pw = "a good long password";
    state
        .db
        .call(move |c| easy_bitcoin_payments::admin_auth::set_password(c, pw, 1_000))
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let n = AtomicU32::new(1);
    let send = |path: &str, session: Option<(&str, &str)>, body: serde_json::Value| {
        let mut r = client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
            .header(
                "x-forwarded-for",
                format!("10.5.0.{}", n.fetch_add(1, Ordering::SeqCst)),
            )
            .json(&body);
        if let Some((cookie, csrf)) = session {
            r = r.header("cookie", cookie).header("x-ebp-csrf", csrf);
        }
        r.send()
    };
    let json = |r: reqwest::Response| async move { r.json::<serde_json::Value>().await.unwrap() };
    let cookie_of = |r: &reqwest::Response| {
        r.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    };

    // Sign in (no second step yet), turn the app code on, add a passkey.
    let login = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    let cookie = cookie_of(&login);
    let csrf = json(login).await["csrf"].as_str().unwrap().to_string();
    let sess = Some((cookie.as_str(), csrf.as_str()));
    let started = json(
        send(
            "/admin/2fa/start",
            sess,
            serde_json::json!({"admin_password": pw}),
        )
        .await
        .unwrap(),
    )
    .await;
    let secret = started["secret"].as_str().unwrap().to_string();
    let done = json(
        send(
            "/admin/2fa/confirm",
            sess,
            serde_json::json!({"code": app_code(&secret, step0)}),
        )
        .await
        .unwrap(),
    )
    .await;
    let codes: Vec<String> = done["recovery_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    let key = SoftKey::new();
    let begun = json(
        send(
            "/admin/passkeys/begin",
            sess,
            serde_json::json!({"admin_password": pw, "code": codes[0]}),
        )
        .await
        .unwrap(),
    )
    .await;
    let challenge = begun["options"]["challenge"].as_str().unwrap().to_string();
    let mut reg = key.registration(&challenge, RP, ORIGIN);
    reg["challenge"] = serde_json::json!(challenge);
    reg["name"] = serde_json::json!("Soft key");
    let r = send("/admin/passkeys/finish", sess, reg).await.unwrap();
    assert_eq!(r.status().as_u16(), 200);

    // The session page says passkeys exist.
    let st = json(
        client
            .get(format!("{base}/admin/session"))
            .header("host", "localhost")
            .header("cookie", &cookie)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(st["passkeys"], true);

    // Wrong password: no passkey options are handed out.
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": "nope nope nope nope"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // Right password, no code: the answer offers the passkey.
    let begin_login = || async {
        let r = send("/admin/login", None, serde_json::json!({"password": pw}))
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        json(r).await
    };
    let first = begin_login().await;
    assert_eq!(first["code_required"], true);
    assert_eq!(first["passkey"]["rpId"], RP);
    assert_eq!(first["passkey"]["allowCredentials"][0]["id"], key.id());
    let ch = first["passkey"]["challenge"].as_str().unwrap().to_string();

    // A passkey this server does not know is refused (400, not "signed out"), and a
    // challenge that was answered once is gone.
    let stranger = SoftKey::new();
    let r = send(
        "/admin/login/passkey",
        None,
        stranger.assertion(&ch, RP, ORIGIN),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send("/admin/login/passkey", None, key.assertion(&ch, RP, ORIGIN))
        .await
        .unwrap();
    assert_eq!(
        r.status().as_u16(),
        400,
        "a used-up challenge stays used up"
    );

    // An answer made for another web address is refused.
    let second = begin_login().await;
    let ch = second["passkey"]["challenge"].as_str().unwrap().to_string();
    let r = send(
        "/admin/login/passkey",
        None,
        key.assertion(&ch, RP, "https://evil.example.net"),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // Without ever typing the password there is nothing to answer.
    let r = send(
        "/admin/login/passkey",
        None,
        key.assertion("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", RP, ORIGIN),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // The right passkey signs in.
    let third = begin_login().await;
    let ch = third["passkey"]["challenge"].as_str().unwrap().to_string();
    let r = send("/admin/login/passkey", None, key.assertion(&ch, RP, ORIGIN))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let cookie2 = cookie_of(&r);
    let csrf2 = json(r).await["csrf"].as_str().unwrap().to_string();
    let sess2 = Some((cookie2.as_str(), csrf2.as_str()));

    // A backup code still signs in too.
    let r = send(
        "/admin/login",
        None,
        serde_json::json!({"password": pw, "code": codes[1]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);

    // Approving a protected action with a passkey. The approval is for the session that
    // asked: another session cannot finish it.
    let sb = json(
        send("/admin/passkeys/stepup/begin", sess2, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(sb["options"]["rpId"], RP);
    let ch = sb["options"]["challenge"].as_str().unwrap().to_string();
    let r = send(
        "/admin/passkeys/stepup/finish",
        sess,
        key.assertion(&ch, RP, ORIGIN),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400, "another session cannot use it");
    let sb = json(
        send("/admin/passkeys/stepup/begin", sess2, serde_json::json!({}))
            .await
            .unwrap(),
    )
    .await;
    let ch = sb["options"]["challenge"].as_str().unwrap().to_string();
    let fin = json(
        send(
            "/admin/passkeys/stepup/finish",
            sess2,
            key.assertion(&ch, RP, ORIGIN),
        )
        .await
        .unwrap(),
    )
    .await;
    let ticket = format!("pk:{}", fin["ticket"].as_str().unwrap());

    // Not signed in: nothing to approve.
    let r = send("/admin/passkeys/stepup/begin", None, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    // The ticket does not skip the password.
    let r = send(
        "/admin/passkeys/begin",
        sess2,
        serde_json::json!({"admin_password": "nope nope nope nope", "code": ticket}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    // With the password it stands in for the code, once.
    let r = send(
        "/admin/passkeys/begin",
        sess2,
        serde_json::json!({"admin_password": pw, "code": ticket}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200, "the passkey approval is accepted");
    let r = send(
        "/admin/passkeys/begin",
        sess2,
        serde_json::json!({"admin_password": pw, "code": ticket}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    assert!(json(r).await.to_string().contains("already used"));
    // A made-up ticket is no better.
    let r = send(
        "/admin/passkeys/begin",
        sess2,
        serde_json::json!({"admin_password": pw, "code": "pk:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);

    // With no passkeys there is nothing to approve with; the backup codes still work
    // for the turn-off buttons.
    let r = send(
        "/admin/passkeys/disable",
        sess2,
        serde_json::json!({"admin_password": pw, "code": codes[2]}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let r = send("/admin/passkeys/stepup/begin", sess2, serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send("/admin/login", None, serde_json::json!({"password": pw}))
        .await
        .unwrap();
    assert_eq!(json(r).await["passkey"], serde_json::Value::Null);
}

/// A tiny barkd stand-in for the wallet step: the create is refused (a wallet
/// is already there), the balance probe answers, and ark-info says signet.
async fn wallet_already_there_barkd() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let (status, body) = if req.starts_with("POST /api/v1/wallet/create") {
                    ("409 Conflict", "{}")
                } else if req.starts_with("GET /api/v1/wallet/ark-info") {
                    ("200 OK", r#"{"network": "signet"}"#)
                } else if req.starts_with("GET /api/v1/wallet/balance") {
                    ("200 OK", r#"{"spendable_sat": 0}"#)
                } else {
                    ("404 Not Found", "{}")
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    base
}

/// A server whose setup has not been done yet, pointed at `barkd_url`.
async fn serve_fresh_setup(barkd_url: String, tried_before: bool) -> (String, AppState) {
    let config = Arc::new(Config {
        barkd_url,
        barkd_token: Some("t".into()),
        public_base_url: "http://localhost:8080".into(),
        ..Config::default()
    });
    let db = Db::open(":memory:").unwrap();
    if tried_before {
        db.call(|c| easy_bitcoin_payments::settings::set(c, "setup_wallet_create_tried", "true"))
            .await
            .unwrap();
    }
    let state = AppState {
        db,
        config,
        rates: Arc::new(RateService::new()),
        wallet: Arc::new(StubWallet::new()),
        webhooks: Arc::new(WebhookEmitter::new(5)),
    };
    let app = router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, state)
}

#[tokio::test]
async fn setup_never_adopts_a_wallet_it_did_not_make() {
    let barkd = wallet_already_there_barkd().await;
    let (base, state) = serve_fresh_setup(barkd, false).await;
    let client = reqwest::Client::new();
    // Asking twice must not adopt it the second time either.
    for _ in 0..2 {
        let resp = client
            .post(format!("{base}/setup/wallet"))
            .header("host", "localhost")
            .json(&serde_json::json!({ "network": "signet" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 409);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(
            body.to_string().contains("did not make"),
            "plain refusal expected: {body}"
        );
    }
    let created = state
        .db
        .call(|c| easy_bitcoin_payments::settings::get_bool(c, "setup_wallet_created"))
        .await
        .unwrap();
    assert!(!created, "a wallet made by someone else is never the till");
    let status: serde_json::Value = client
        .get(format!("{base}/setup/status"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["wallet_created"], false, "{status}");
}

#[tokio::test]
async fn setup_resumes_with_a_wallet_its_own_earlier_attempt_made() {
    // The first create went through at barkd but the answer was lost.
    let barkd = wallet_already_there_barkd().await;
    let (base, state) = serve_fresh_setup(barkd, true).await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/setup/wallet"))
        .header("host", "localhost")
        .json(&serde_json::json!({ "network": "signet" }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let created = state
        .db
        .call(|c| easy_bitcoin_payments::settings::get_bool(c, "setup_wallet_created"))
        .await
        .unwrap();
    assert!(created);
}

#[tokio::test]
async fn running_the_shop_step_again_never_changes_payout_or_password() {
    const FIRST: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
    const OTHER: &str = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7";
    let (base, state) = serve_fresh_setup("http://127.0.0.1:9/api/v1".into(), false).await;
    state
        .db
        .call(|c| {
            store::create(c, "Shop", None)?;
            easy_bitcoin_payments::payout::set_initial(c, FIRST, "test")?;
            easy_bitcoin_payments::admin_auth::set_password(c, "the first long password 1", 1_000)
        })
        .await
        .unwrap();
    let read = || async {
        state
            .db
            .call(|c| {
                Ok((
                    easy_bitcoin_payments::payout::effective(c, None)?,
                    easy_bitcoin_payments::admin_auth::stored_hash(c)?,
                ))
            })
            .await
            .unwrap()
    };
    let before = read().await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/setup/store"))
        .header("host", "localhost")
        .json(&serde_json::json!({
            "name": "Shop",
            "payout_address": OTHER,
            "admin_password": "a long new password 4821"
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let after = read().await;
    assert_eq!(after.0.as_deref(), Some(FIRST), "payout address unchanged");
    assert_eq!(after.1, before.1, "admin password unchanged");
}

/// A barkd stand-in that only answers ark-info, with this server key.
async fn ark_info_barkd(server_pubkey: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let body = if req.starts_with("GET /api/v1/wallet/ark-info") {
                    format!(r#"{{"network": "signet", "server_pubkey": "{server_pubkey}"}}"#)
                } else {
                    "{}".to_string()
                };
                let status = if body == "{}" {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    base
}

#[tokio::test]
async fn an_ark_address_for_another_ark_server_is_refused_when_saved() {
    // Second's server key: the address below was made for a different server.
    let barkd =
        ark_info_barkd("0375b2e4f6abe5b00736359de211c35d9e72b615e5fd424d8ad1e68a8b300b97b5").await;
    let config = Config {
        password_hash_iterations: 1_000,
        barkd_url: barkd,
        barkd_token: Some("t".into()),
        ..Config::default()
    };
    let (base, state, _store_id, _key) = serve_with(config).await;
    state
        .db
        .call(|c| easy_bitcoin_payments::admin_auth::set_password(c, "a good long password", 1_000))
        .await
        .unwrap();
    let (cookie, csrf) = owner_session(&base, &state).await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/admin/payout"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .json(&serde_json::json!({
            "address": "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh",
            "admin_password": "a good long password"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body = resp.text().await.unwrap();
    assert!(body.contains("different Ark server"), "{body}");
}

#[tokio::test]
async fn a_restore_whose_answer_was_lost_carries_on_and_shows_the_words_again() {
    const WORDS: &str =
        "abandon ability able about above absent absorb abstract absurd abuse access accident";
    let restore = serde_json::json!({
        "network": "signet", "mnemonic": WORDS, "paper_copies_exist": true
    });
    // Never tried here before: a wallet that is already there is not restored over.
    let (base, _state) = serve_fresh_setup(wallet_already_there_barkd().await, false).await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/setup/wallet"))
        .header("host", "localhost")
        .json(&restore)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 409);

    // This wizard's own earlier restore went through, but its answer was lost.
    let (base, state) = serve_fresh_setup(wallet_already_there_barkd().await, true).await;
    let resp = reqwest::Client::new()
        .post(format!("{base}/setup/wallet"))
        .header("host", "localhost")
        .json(&restore)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let (created, restored, confirmed) = state
        .db
        .call(|c| {
            use easy_bitcoin_payments::settings::get_bool;
            Ok((
                get_bool(c, "setup_wallet_created")?,
                get_bool(c, "setup_wallet_restored")?,
                get_bool(c, "setup_backup_confirmed")?,
            ))
        })
        .await
        .unwrap();
    assert!(created && restored);
    assert!(
        !confirmed,
        "the words are shown again to check against the paper copy"
    );
}

#[tokio::test]
async fn owner_and_api_answers_are_never_cached() {
    let (base, state, _store, key) = serve().await;
    let (cookie, csrf) = owner_session(&base, &state).await;
    let client = reqwest::Client::new();
    let r = client
        .get(format!("{base}/api/v1/invoices"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["cache-control"], "no-store");
    let r = client
        .get(format!("{base}/admin/session"))
        .header("host", "localhost")
        .header("cookie", &cookie)
        .header("x-ebp-csrf", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.headers()["cache-control"], "no-store");
}

/// A barkd stand-in that answers only the next-round question.
async fn next_round_barkd(start_time: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let (status, body) = if req.starts_with("GET /api/v1/wallet/next-round") {
                    ("200 OK", format!(r#"{{"start_time": "{start_time}"}}"#))
                } else {
                    ("404 Not Found", "{}".to_string())
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    base
}

#[tokio::test]
async fn home_gets_the_next_round_time_only_while_a_renewal_is_waiting_for_one() {
    let client = reqwest::Client::new();
    let (base, state, _store, _key) = serve_with(Config {
        admin_token: Some("adm1n".to_string()),
        barkd_url: next_round_barkd("2026-11-21T22:00:00Z").await,
        barkd_token: Some("t".into()),
        ..Config::default()
    })
    .await;
    let status = || async {
        client
            .get(format!("{base}/api/v1/admin/status"))
            .header("host", "localhost")
            .bearer_auth("adm1n")
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()
    };

    // Nothing waiting: no time is asked for or shown.
    let st = status().await;
    assert_eq!(st["counters"]["renewals_waiting"], 0);
    assert!(st["wallet"]["next_round_at"].is_null());

    // A renewal asked for and not finished: the next round time comes with the status.
    state
        .db
        .call(|c| {
            c.execute(
                "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at)
                 VALUES ('r1','renew',4000,'','requested','asked to renew 1 coins in the Ark server''s next round',1000)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let st = status().await;
    assert_eq!(st["counters"]["renewals_waiting"], 1);
    assert_eq!(st["wallet"]["next_round_at"], "2026-11-21T22:00:00Z");
}
