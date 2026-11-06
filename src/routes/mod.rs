pub mod admin;
pub mod admin_session;
pub mod alerts;
pub mod backup;
pub mod checkout;
pub mod checkout_rails;
pub mod dashboard;
pub mod deliveries;
pub mod health;
pub mod invoices;
pub mod payout;
pub mod setup;
pub mod stores;
pub mod unmatched;

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
        .route("/setup/wallet/verify-backup", post(setup::verify_backup))
        .route("/setup/verify-code", post(setup::verify_code))
        .route("/setup/token", post(setup::set_token))
        .route("/setup/store", post(setup::create_store))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            setup::setup_token_guard,
        ))
        // Brute-force brake on the setup code: per-client request cap on the
        // whole wizard (60/min is far above real use and far below guessing a
        // 59-bit code).
        .layer(middleware::from_fn_with_state(
            Arc::new(crate::ratelimit::RateLimiter::new(
                60,
                state.config.trust_proxy_headers,
            )),
            crate::ratelimit::limit,
        ))
        .with_state(state.clone());

    // Public checkout routes (unauthenticated) get a per-client rate limit
    // (beta hardening; see ratelimit.rs). Merged into `gated` below so they
    // still sit behind the setup gate.
    let limiter = Arc::new(crate::ratelimit::RateLimiter::new(
        state.config.public_rate_limit_per_min,
        state.config.trust_proxy_headers,
    ));
    let public_checkout = Router::new()
        .route("/i/{id}", get(checkout::page))
        // The checkout page's own first data fetch (assets/checkout.html
        // calls /i/{id}/json before polling /status) — this route existed
        // only inside a never-merged router() until the overnight review
        // caught the 404.
        .route("/i/{id}/json", get(checkout::invoice_json))
        .route("/i/{id}/status", get(checkout::status))
        .route("/i/{id}/rails", get(checkout_rails::rails))
        .layer(middleware::from_fn_with_state(
            limiter,
            crate::ratelimit::limit,
        ));

    // Invoice API + the rate-limited checkout routes sit behind the setup gate.
    let gated = Router::new()
        .merge(public_checkout)
        .route(
            "/api/v1/invoices",
            post(invoices::create).get(invoices::list),
        )
        .route("/api/v1/invoices/{id}", get(invoices::get_one))
        .route("/api/v1/invoices/{id}/cancel", post(invoices::cancel))
        .route("/api/v1/admin/status", get(admin::status))
        .route(
            "/api/v1/stores/webhook",
            axum::routing::put(stores::set_webhook),
        )
        .route("/api/v1/admin/version", get(stores::version))
        .route("/api/v1/payout", get(payout::get))
        .route("/api/v1/stores/webhook/test", post(stores::test_webhook))
        .route("/api/v1/stores/rotate-key", post(stores::rotate_key))
        .route("/api/v1/webhook-deliveries", get(deliveries::list))
        .route(
            "/api/v1/webhook-deliveries/{id}/resend",
            post(deliveries::resend),
        )
        .route("/api/v1/audit-log", get(deliveries::audit))
        .route("/api/v1/unmatched-payments", get(unmatched::list))
        .route(
            "/api/v1/unmatched-payments/{id}/resolve",
            post(unmatched::resolve),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            setup::setup_gate,
        ))
        .with_state(state.clone());

    Router::new()
        .route("/", get(dashboard::root))
        .route("/dashboard", get(dashboard::dashboard))
        .route("/admin", get(dashboard::dashboard))
        .route("/admin/session", get(admin_session::session))
        .route("/admin/logout", post(admin_session::logout))
        .route("/admin/payout/cancel", post(payout::cancel))
        .route("/admin/sweep", post(payout::sweep_now))
        .route("/admin/alerts/status", post(alerts::get))
        .route("/admin/alerts/enable", post(alerts::enable))
        .route("/admin/alerts/disable", post(alerts::disable))
        .route("/admin/alerts/test", post(alerts::test))
        .route("/admin/password", post(admin_session::change_password))
        .merge(
            Router::new()
                .route("/admin/login", post(admin_session::login))
                // Checks the admin password again, so it shares the brake.
                .route("/admin/backup", post(backup::download))
                // Also checks the admin password.
                .route("/admin/payout", post(payout::change))
                // Password-guessing brake: 10 attempts a minute per client.
                .layer(middleware::from_fn_with_state(
                    Arc::new(crate::ratelimit::RateLimiter::new(
                        10,
                        state.config.trust_proxy_headers,
                    )),
                    crate::ratelimit::limit,
                ))
                // ...and an overall cap that does not depend on trusting any
                // client address (covers a misconfigured proxy setup).
                .layer(middleware::from_fn_with_state(
                    Arc::new(crate::ratelimit::RateLimiter::new_global(120)),
                    crate::ratelimit::limit,
                ))
                .with_state(state.clone()),
        )
        .route("/assets/qr.js", get(checkout::qr_js))
        .route("/api/v1/health", get(health::health))
        .merge(setup)
        .merge(gated)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

/// Baseline browser-hardening headers on EVERY response (found missing in
/// the beta review). Matters most for `/setup` (shows the recovery words
/// and the API key) and the dashboard: `frame-ancestors 'none'` +
/// `X-Frame-Options: DENY` stop clickjacking, `nosniff` stops content
/// sniffing, `no-referrer` keeps invoice/setup URLs out of Referer headers.
/// The CSP matches the static pages as built: same-origin fetches, inline
/// script/style (no build step), no third-party resources. If a page ever
/// loads an external asset, widen this deliberately. Handlers that set their
/// own value for a header keep it (we only fill gaps).
async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::HeaderValue;
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
             connect-src 'self'; base-uri 'none'; form-action 'self'; \
             frame-ancestors 'none'",
        ),
    ] {
        headers
            .entry(axum::http::HeaderName::from_static(name))
            .or_insert(HeaderValue::from_static(value));
    }
    response
}
