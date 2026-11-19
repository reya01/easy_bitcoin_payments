//! Public checkout routes (unauthenticated). The checkout offers two ways to
//! pay: Lightning (the default) and Ark. EBP takes no regular (on-chain)
//! Bitcoin payments and shows no combined "one code" link.

use axum::{
    extract::{Path, State},
    response::Html,
    Json,
};
use rusqlite::OptionalExtension;

use crate::{error::AppError, invoice, routes::AppState};

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

/// The success-screen mascot art (`assets/mascot/*.png`). The pictures are compiled into the
/// program and served from this fixed list only: there is deliberately no way to add, replace or
/// upload an image while the server runs, and any other name is a plain 404. GET only (the
/// router answers every other method with 405). Public, cacheable.
pub async fn mascot(
    Path(name): Path<String>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let bytes: &'static [u8] = match name.as_str() {
        "sir-body.png" => include_bytes!("../../assets/mascot/sir-body.png"),
        "sir-arm.png" => include_bytes!("../../assets/mascot/sir-arm.png"),
        "ufo-top.png" => include_bytes!("../../assets/mascot/ufo-top.png"),
        "ufo-beam.png" => include_bytes!("../../assets/mascot/ufo-beam.png"),
        "fancy-body.png" => include_bytes!("../../assets/mascot/fancy-body.png"),
        "fancy-hat.png" => include_bytes!("../../assets/mascot/fancy-hat.png"),
        "acro.png" => include_bytes!("../../assets/mascot/acro.png"),
        _ => return Err(AppError::NotFound),
    };
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "image/png"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    ))
}

pub async fn page() -> Html<&'static str> {
    Html(include_str!("../../assets/checkout.html"))
}

/// One payload for everything the page renders once.
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
    let usd_per_btc = app.usd_per_btc();
    Ok(Json(serde_json::json!({
        "return_url": return_url,
        "store_name": store_name, "status": inv.status.as_str(), "amount_sats": inv.amount_sats,
        "fiat_currency": inv.fiat_currency, "fiat_amount": inv.fiat_amount,
        "ark_address": inv.ark_address, "expires_at_unix": inv.expires_at,
        "lightning": bolt11.map(|b| serde_json::json!({ "bolt11": b, "fee_estimate_sat": fee })),
        "test_mode": app.config.mode == "test",
        // For showing "(about $0.40)" next to sats; null when no price is known.
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
    // Part-paid invoices: how much has arrived, how much is still missing and
    // whether a top-up is still accepted, so the page can say so honestly.
    // Additive: older pages ignore these fields.
    let underpaid = inv.status == crate::models::InvoiceStatus::Underpaid;
    let topup_closes_at = if underpaid {
        let inv_for_window = inv.clone();
        app.db
            .call(move |c| invoice::topup_closes_at(c, &inv_for_window))
            .await?
    } else {
        None
    };
    let now = crate::util::now_unix();
    Ok(Json(serde_json::json!({
        "status": inv.status.as_str(),
        "amount_sats": inv.amount_sats,
        "received_sats": inv.paid_sats,
        "remaining_sats": underpaid.then(|| (inv.amount_sats - inv.paid_sats).max(0)),
        "topup_window_open": underpaid.then(|| topup_closes_at.is_some_and(|t| now <= t)),
        "topup_closes_at_unix": topup_closes_at,
        "expires_at_unix": inv.expires_at,
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
