//! Route-wiring regression test: the checkout page's own JavaScript fetches
//! specific URLs (`/i/{id}/json`, `/i/{id}/status`); a route that exists in
//! a handler module but is never MERGED into the served router 404s only at
//! runtime, on a tester's phone. This test serves the real composed router
//! on a loopback port and asserts the page-facing routes actually answer,
//! that the setup gate holds, and that the API surface stays authenticated.
//! (Found 2026-11-03: `/i/{id}/json` was wired only inside a dead
//! `router()` that no one ever merged — the checkout page would 404 its
//! first data fetch.)

use std::collections::HashMap;
use std::sync::Arc;

use tokio::net::TcpListener;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::invoice::{self, NewInvoice};
use easy_bitcoin_payments::rates::StaticRateProvider;
use easy_bitcoin_payments::routes::{router, AppState};
use easy_bitcoin_payments::store;
use easy_bitcoin_payments::util::now_unix;
use easy_bitcoin_payments::wallet::StubWallet;
use easy_bitcoin_payments::webhook::WebhookEmitter;

async fn test_state() -> AppState {
    let dir = std::env::temp_dir().join(format!("ebp-routes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("routes-test.sqlite3");
    let _ = std::fs::remove_file(&db_path);
    let config = Arc::new(Config {
        database_path: db_path.to_string_lossy().to_string(),
        ..Config::default()
    });
    let db = Db::open(&config.database_path).unwrap();
    db.call(|conn| store::create(conn, "Wiring Shop", None))
        .await
        .unwrap();
    AppState {
        db,
        config,
        rates: Arc::new(StaticRateProvider::new(HashMap::new())),
        usd: Arc::new(easy_bitcoin_payments::rates::UsdDisplay::off()),
        wallet: Arc::new(StubWallet::new()),
        webhooks: Arc::new(WebhookEmitter::new(5)),
    }
}

async fn insert_test_invoice(state: &AppState) {
    let store_id = state
        .db
        .call(|conn| {
            conn.query_row("SELECT id FROM stores LIMIT 1", [], |r| {
                r.get::<_, String>(0)
            })
            .map_err(easy_bitcoin_payments::error::AppError::from)
        })
        .await
        .unwrap();
    state
        .db
        .call(move |conn| {
            invoice::insert(
                conn,
                &NewInvoice {
                    id: "inv_wiring".to_string(),
                    store_id,
                    order_id: Some("wc-9".to_string()),
                    amount_sats: 1000,
                    fiat_currency: None,
                    fiat_amount: None,
                    rate: None,
                    rate_source: None,
                    rate_locked_at: None,
                    ark_address: "tark1qwiring".to_string(),
                    onchain_address: "bc1qwiring".to_string(),
                    expires_at: now_unix() + 1200,
                    confs_required: 1,
                    metadata_json: "{}".to_string(),
                },
            )
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn page_facing_routes_actually_answer_on_the_served_router() {
    let state = test_state().await;
    insert_test_invoice(&state).await;
    let client = reqwest::Client::new();

    // 0. PRE-setup: the gate must hold — checkout routes answer 503
    //    SetupIncomplete before the wizard finishes.
    let pre_app = router(state.clone());
    let pre_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pre_base = format!("http://{}", pre_listener.local_addr().unwrap());
    let pre_server = tokio::spawn(async move {
        let _ = axum::serve(pre_listener, pre_app).await;
    });
    let gated_status = client
        .get(format!("{pre_base}/i/inv_wiring/json"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(
        gated_status.status(),
        503,
        "pre-setup checkout MUST be gated (503 SetupIncomplete)"
    );
    pre_server.abort();

    // Simulate the wizard completing (wallet + backup + token verified; the
    // store already exists), then serve the real router.
    state
        .db
        .call(|conn| {
            easy_bitcoin_payments::settings::set(conn, "setup_wallet_created", "true")?;
            easy_bitcoin_payments::settings::set(conn, "setup_backup_confirmed", "true")?;
            easy_bitcoin_payments::settings::set(conn, "setup_token_verified", "true")?;
            // the wizard's recompute_setup_complete stores this flag itself
            easy_bitcoin_payments::settings::set(conn, "setup_complete", "true")
        })
        .await
        .unwrap();

    let app = router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // 1. The checkout page's first data fetch — the route the overnight
    //    review found unwired. Must be 200 and JSON.
    let body: serde_json::Value = client
        .get(format!("{base}/i/inv_wiring/json"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .expect("/i/{id}/json must answer 200 — the checkout page's first fetch")
        .json()
        .await
        .unwrap();
    assert_eq!(body["amount_sats"], 1000);
    assert!(body["bip321"]
        .as_str()
        .map(|s| s.starts_with("bitcoin:"))
        .unwrap_or(false));

    // 2. The checkout page itself serves (200 HTML).
    let page = client
        .get(format!("{base}/i/inv_wiring"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert!(page.status().is_success(), "checkout page must serve");
    assert!(
        page.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/html"))
            .unwrap_or(false),
        "checkout page must be HTML"
    );

    // 3. Status route answers.
    let status = client
        .get(format!("{base}/i/inv_wiring/status"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert!(status.status().is_success());

    // 4. The merchant API stays authenticated: no key -> 401, never a leak.
    let unauthorized = client
        .get(format!("{base}/api/v1/invoices"))
        .header("host", "localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);

    server.abort();
}
