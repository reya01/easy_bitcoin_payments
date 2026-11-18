//! The scripts and stylesheets of the three pages (checkout, admin, setup).
//!
//! They used to sit inside the HTML, which forced the Content-Security-Policy
//! to allow inline code (`'unsafe-inline'`). Served as separate files from the
//! same server, the policy can be `script-src 'self'; style-src 'self'`: a
//! script that somehow got injected into a page would not run. The files are
//! compiled into the program and served from this fixed list only.
//!
//! `no-cache` makes the browser check for a newer copy on every page load, so
//! after an upgrade a page never runs yesterday's script with today's HTML.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;

const JS: &str = "application/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";

fn file(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-cache")],
        body,
    )
}

pub async fn checkout_js() -> impl IntoResponse {
    file(JS, include_str!("../../assets/checkout.js"))
}

pub async fn checkout_css() -> impl IntoResponse {
    file(CSS, include_str!("../../assets/checkout.css"))
}

pub async fn dashboard_js() -> impl IntoResponse {
    file(JS, include_str!("../../assets/dashboard.js"))
}

pub async fn dashboard_css() -> impl IntoResponse {
    file(CSS, include_str!("../../assets/dashboard.css"))
}

pub async fn setup_js() -> impl IntoResponse {
    file(JS, include_str!("../../assets/setup.js"))
}

pub async fn setup_css() -> impl IntoResponse {
    file(CSS, include_str!("../../assets/setup.css"))
}
