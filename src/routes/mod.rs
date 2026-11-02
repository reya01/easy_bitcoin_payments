pub mod checkout;
pub mod checkout_rails;
pub mod dashboard;
pub mod health;
pub mod invoices;
pub mod setup;

use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::db::Db;
use crate::rates::RateProvider;
use crate::wallet::WalletAdapter;
use crate::webhook::WebhookEmitter;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub rates: Arc<dyn RateProvider>,
    pub wallet: Arc<dyn WalletAdapter>,
    pub webhooks: Arc<WebhookEmitter>,
}

pub fn router(state: AppState) -> Router {
    use axum::middleware;

    // Setup wizard: reachable while setup is incomplete (the wizard guards
    // itself; see setup::ensure_wizard_open). Status stays readable after.
    let setup = Router::new()
        .route("/setup", get(setup::page))
        .route("/setup/status", get(setup::status))
        .route("/setup/wallet", post(setup::create_wallet))
        .route("/setup/wallet/confirm-backup", post(setup::confirm_backup))
        .route("/setup/wallet/mnemonic", post(setup::wallet_mnemonic))
        .route("/setup/token", post(setup::set_token))
        .route("/setup/store", post(setup::create_store))
        .with_state(state.clone());

    // Invoice + checkout routes sit behind the setup gate.
    let gated = Router::new()
        .route("/i/{id}", get(checkout::page))
        .route("/i/{id}/status", get(checkout::status))
        .route("/i/{id}/rails", get(checkout_rails::rails))
        .route(
            "/api/v1/invoices",
            post(invoices::create).get(invoices::list),
        )
        .route("/api/v1/invoices/{id}", get(invoices::get_one))
        .route("/api/v1/invoices/{id}/cancel", post(invoices::cancel))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            setup::setup_gate,
        ))
        .with_state(state.clone());

    Router::new()
        .route("/", get(dashboard::root))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/api/v1/health", get(health::health))
        .merge(setup)
        .merge(gated)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
