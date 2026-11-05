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
            invoice::record_unmatched(conn, "ark1qlate", "ark", 1000, Some("v_late"), None, 1)?;
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
