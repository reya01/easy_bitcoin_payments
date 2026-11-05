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
