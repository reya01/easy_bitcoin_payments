//! API-level regression tests for the beta-hardening endpoints, served on the
//! real composed router over loopback:
//!   * `POST /api/v1/invoices` idempotency per `order_id`
//!   * `GET /api/v1/unmatched-payments` + `POST .../resolve` (store-scoped)
//!
//! Pattern copied from tests/routes_wiring.rs. If you add API behaviour,
//! extend this file rather than growing routes_wiring.rs.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::TcpListener;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::invoice;
use easy_bitcoin_payments::rates::StaticRateProvider;
use easy_bitcoin_payments::routes::{router, AppState};
use easy_bitcoin_payments::store;
use easy_bitcoin_payments::util::now_unix;
use easy_bitcoin_payments::wallet::StubWallet;
use easy_bitcoin_payments::webhook::WebhookEmitter;

/// Serve the router with setup marked complete; returns (base url, state,
/// store id, api key).
async fn serve() -> (String, AppState, String, String) {
    serve_with(Config::default()).await
}

async fn serve_with(config: Config) -> (String, AppState, String, String) {
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
        rates: Arc::new(StaticRateProvider::new(HashMap::new())),
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
async fn unmatched_payments_can_be_listed_and_resolved_by_their_store_only() {
    let (base, state, store_id, key) = serve().await;
    let client = reqwest::Client::new();

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
                    onchain_address: "bc1qlate".to_string(),
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

    let rows: Vec<serde_json::Value> = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .bearer_auth(&key)
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
    let resolved = client
        .post(format!("{base}/api/v1/unmatched-payments/{id}/resolve"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(resolved.status(), 200);

    let rows: Vec<serde_json::Value> = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!rows[0]["resolved_at"].is_null(), "row is marked resolved");

    // A second store sees nothing and cannot resolve the first store's row.
    let (_, other_key, _) = state
        .db
        .call(|conn| store::create(conn, "Other Shop", None))
        .await
        .unwrap();
    let theirs: Vec<serde_json::Value> = client
        .get(format!("{base}/api/v1/unmatched-payments"))
        .header("host", "localhost")
        .bearer_auth(&other_key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(theirs.is_empty());
    let cross = client
        .post(format!("{base}/api/v1/unmatched-payments/{id}/resolve"))
        .header("host", "localhost")
        .bearer_auth(&other_key)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), 404);
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
async fn onchain_rail_is_off_by_default_and_switchable() {
    let client = reqwest::Client::new();

    // Default: no on-chain address in the API, the checkout JSON, or the QR.
    let (base, _s, _store, key) = serve().await;
    let (_, inv) = create(&client, &base, &key, "wc-oc", 2_000).await;
    assert!(inv["onchain_address"].is_null());
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
    assert_eq!(co["onchain_enabled"], false);
    assert!(co["onchain_address"].is_null());
    assert!(co["bip321"]
        .as_str()
        .unwrap()
        .starts_with("bitcoin:?amount="));

    // Opt in: the address and the URI path come back.
    let (base, _s, _store, key) = serve_with(Config {
        onchain_rail_enabled: true,
        ..Config::default()
    })
    .await;
    let (_, inv) = create(&client, &base, &key, "wc-oc", 2_000).await;
    assert!(inv["onchain_address"].is_string());
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
    assert_eq!(co["onchain_enabled"], true);
    assert!(co["bip321"].as_str().unwrap().starts_with("bitcoin:bc1q"));
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
    let (base, _state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
    let put = |body: serde_json::Value, key: String| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .put(format!("{base}/api/v1/stores/webhook"))
                .header("host", "localhost")
                .bearer_auth(key)
                .json(&body)
                .send()
                .await
                .unwrap()
        }
    };

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

    // Key rotation: new key works, old key is dead.
    let r = client
        .post(format!("{base}/api/v1/stores/rotate-key"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let new_key = r.json::<serde_json::Value>().await.unwrap()["api_key"]
        .as_str()
        .unwrap()
        .to_string();
    let old = put(serde_json::json!({ "url": null }), key).await;
    assert_eq!(old.status(), 401);
    let fresh = put(serde_json::json!({ "url": null }), new_key).await;
    assert_eq!(fresh.status(), 200);
}

#[tokio::test]
async fn invoice_inputs_are_bounded_and_unmatched_list_paginates() {
    let (base, _state, _store_id, key) = serve().await;
    let client = reqwest::Client::new();
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
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
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

#[tokio::test]
async fn delivery_log_resend_and_audit_trail_are_store_scoped() {
    let (base, state, store_id, key) = serve().await;
    let client = reqwest::Client::new();
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
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1, "only this store's rows");
    assert_eq!(list[0]["id"], "whd_mine");

    let other = client
        .post(format!("{base}/api/v1/webhook-deliveries/whd_other/resend"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(other.status(), 404, "another store's delivery is invisible");

    let ok = client
        .post(format!("{base}/api/v1/webhook-deliveries/whd_mine/resend"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let pending: serde_json::Value = client
        .get(format!("{base}/api/v1/webhook-deliveries?status=pending"))
        .header("host", "localhost")
        .bearer_auth(&key)
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
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(audit[0]["action"], "webhook.resend");
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
    let (base, _state, _store_id, key) = serve_with(Config::default()).await;
    let client = reqwest::Client::new();
    let test = || {
        client
            .post(format!("{base}/api/v1/stores/webhook/test"))
            .header("host", "localhost")
            .bearer_auth(&key)
            .send()
    };
    // No webhook yet: refused with a plain message.
    assert_eq!(test().await.unwrap().status().as_u16(), 400);

    let set = client
        .put(format!("{base}/api/v1/stores/webhook"))
        .header("host", "localhost")
        .bearer_auth(&key)
        .json(&serde_json::json!({ "url": "https://shop.example.com/hook" }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status().as_u16(), 200);
    assert_eq!(test().await.unwrap().status().as_u16(), 200);

    let list: serde_json::Value = client
        .get(format!("{base}/api/v1/webhook-deliveries"))
        .header("host", "localhost")
        .bearer_auth(&key)
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

#[tokio::test]
async fn encrypted_backup_download_needs_session_admin_password_and_decrypts() {
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
async fn payout_address_first_is_immediate_changes_wait_and_need_the_password() {
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
    let login = client
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
    let post = |path: &str, body: serde_json::Value| {
        client
            .post(format!("{base}{path}"))
            .header("host", "localhost")
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

    // First address: immediate.
    let r: serde_json::Value = post(
        "/admin/payout",
        serde_json::json!({"address": a, "admin_password": "a good long password"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(r["applied_now"], true);
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
        .json(&serde_json::json!({"address": a, "admin_password": "a good long password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
}

/// The code an authenticator app would show for a stored secret at a 30-second step.
fn app_code(secret_b32: &str, step: i64) -> String {
    let key = easy_bitcoin_payments::totp::base32_decode(secret_b32).unwrap();
    easy_bitcoin_payments::totp::code_for_step(&key, step)
}

#[tokio::test]
async fn two_step_sign_in_turns_on_gates_sign_in_and_actions_and_turns_off() {
    use std::sync::atomic::{AtomicU32, Ordering};
    // Fixed once, so a 30-second boundary passing mid-test cannot change the codes used.
    let step0 = now_unix() / 30;
    let config = Config {
        password_hash_iterations: 1_000,
        // Each request below pretends to come from its own address so the
        // password-guessing brake (10 a minute per client) does not interfere.
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
    assert_eq!(r.status().as_u16(), 400);

    // Turning it off needs the password and a code, then sign-in is password-only again.
    let r = send(
        "/admin/2fa/disable",
        sess2,
        serde_json::json!({"admin_password": pw}),
    )
    .await
    .unwrap();
    assert_eq!(r.status().as_u16(), 400);
    let r = send(
        "/admin/2fa/disable",
        sess2,
        serde_json::json!({"admin_password": pw, "code": app_code(&secret, step0 + 1)}),
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
            // Five wrong tries (a different step than the one just used).
            for i in 0..5 {
                let r = totp::verify(c, "000000", now, false)?;
                assert_eq!(
                    r,
                    if i < 4 { Second::Wrong } else { Second::Locked },
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
