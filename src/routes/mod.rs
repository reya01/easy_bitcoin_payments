pub mod checkout;
pub mod dashboard;
pub mod health;
pub mod invoices;

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
    Router::new()
        .route("/", get(dashboard::root))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/i/{id}", get(checkout::page))
        .route("/i/{id}/status", get(checkout::status))
        .route(
            "/api/v1/invoices",
            post(invoices::create).get(invoices::list),
        )
        .route("/api/v1/invoices/{id}", get(invoices::get_one))
        .route("/api/v1/invoices/{id}/cancel", post(invoices::cancel))
        .route("/api/v1/health", get(health::health))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}