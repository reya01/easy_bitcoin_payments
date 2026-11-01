use axum::response::{Html, Redirect};

const DASHBOARD_HTML: &str = include_str!("../../assets/dashboard.html");

pub async fn root() -> Redirect {
    Redirect::to("/dashboard")
}

/// Merchant dashboard placeholder.
///
/// Phase 0 ships the route and the mobile-first shell so the deploy path and
/// navigation are real. Balances, history, the expiry queue, ASP status, and
/// the accounting export land in Phase 1/2.
pub async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}
