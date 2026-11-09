//! Public checkout routes (unauthenticated). Byte Store drill 2026-11-02:
//! the default QR is the combined BIP321 URI (crate::uri) — on-chain path
//! + ark/lightning params per enabled rail; per-rail tabs are copies.

use axum::{
    extract::{Path, State},
    response::Html,
    Json,
};
use rusqlite::OptionalExtension;

use crate::{error::AppError, invoice, routes::AppState, uri};

/// The vendored QR generator (`assets/qr.js`), served same-origin so the
/// page CSP (`script-src 'self'`) holds without a CDN. Public, cacheable.
pub async fn qr_js() -> impl axum::response::IntoResponse {
    (
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        include_str!("../../assets/qr.js"),
    )
}

pub async fn page() -> Html<&'static str> {
    Html(include_str!("../../assets/checkout.html"))
}

/// One payload for everything the page renders once, incl. the default-QR URI.
pub async fn invoice_json(
    Path(id): Path<String>,
    State(app): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (inv, store_name) = app
        .db
        .call(move |c| invoice::get_public(c, &id))
        .await?
        .ok_or(AppError::NotFound)?;
    let inv_id = inv.id.clone();
    let bolt11 = app.db.call(move |c| ln_bolt11(c, &inv_id)).await?;
    let fee = serde_json::from_str::<serde_json::Value>(&inv.metadata_json)
        .ok()
        .and_then(|m| m.get("lightning_fee_estimate_sat").and_then(|v| v.as_i64()));
    let return_url = serde_json::from_str::<serde_json::Value>(&inv.metadata_json)
        .ok()
        .and_then(|m| {
            m.get("return_url")
                .and_then(|v| v.as_str().map(str::to_owned))
        })
        .filter(|u| crate::routes::invoices::valid_return_url(u));
    let onchain_enabled = app.config.onchain_rail_enabled;
    let usd_per_btc = app.usd_per_btc().await;
    Ok(Json(serde_json::json!({
        "return_url": return_url,
        "store_name": store_name, "status": inv.status.as_str(), "amount_sats": inv.amount_sats,
        "fiat_currency": inv.fiat_currency, "fiat_amount": inv.fiat_amount,
        "ark_address": inv.ark_address, "expires_at_unix": inv.expires_at,
        // On-chain rail is OFF by default (config.onchain_rail_enabled): no
        // address, no URI path, and the page hides the tab.
        "onchain_enabled": onchain_enabled,
        "onchain_address": onchain_enabled.then(|| inv.onchain_address.clone()),
        "bip321": uri::combined(inv.amount_sats, onchain_enabled.then_some(inv.onchain_address.as_str()), bolt11.as_deref(), Some(&inv.ark_address)),
        "lightning": bolt11.map(|b| serde_json::json!({ "bolt11": b, "fee_estimate_sat": fee })),
        "test_mode": app.config.mode == "test",
        // For showing "(about $0.40)" next to sats; null when unknown or turned off.
        "usd_per_btc": usd_per_btc,
    })))
}

/// Polled ~1/s while pending (store bundle: te=1e3 ms); the page stops on paid/expired.
pub async fn status(
    Path(id): Path<String>,
    State(app): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let inv = app
        .db
        .call(move |c| invoice::expire_if_due(c, &id))
        .await?
        .ok_or(AppError::NotFound)?;
    // On-chain payment seen but not yet deep enough: the page says so
    // instead of looking stuck (or, worse, showing "paid" too early).
    let inv_id = inv.id.clone();
    let seen_depth = if inv.status == crate::models::InvoiceStatus::Pending {
        app.db
            .call(move |c| invoice::onchain_seen_depth(c, &inv_id))
            .await?
    } else {
        None
    };
    Ok(Json(serde_json::json!({
        "status": inv.status.as_str(),
        "onchain_confirmations": seen_depth,
        "onchain_confirmations_required": app.config.onchain_min_confirmations,
    })))
}

/// The BOLT11 issued for this invoice, if any. (The Byte Store correlates
/// LN payments by payment_hash, and barkd's history metadata carries ours
/// too — a future option for LN-native attribution. No change now: EBP
/// attributes by till-issued addresses/VTXOs.)
fn ln_bolt11(conn: &rusqlite::Connection, invoice_id: &str) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT bolt11 FROM ln_invoices WHERE invoice_id = ?1",
        [invoice_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(Into::into)
}
