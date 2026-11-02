//! Public payment-rail details for the checkout page.
//!
//! The checkout status endpoint (src/routes/checkout.rs) predates
//! Lightning receive and the test-mode banner; rather than grow that
//! payload's shape (and its consumers) in this pass, the new fields ride
//! on a sibling endpoint the checkout page fetches once. Everything here
//! is public, like the checkout itself: no secrets, no PII — just what a
//! customer needs to choose a rail.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;

use crate::error::AppError;
use crate::invoice;
use crate::routes::AppState;

#[derive(Debug, Serialize)]
pub struct LightningRail {
    /// BOLT11 invoice from the ASP's Lightning gateway, when one exists.
    pub bolt11: String,
    /// Gateway fee estimate in sats, fetched at invoice creation. `null`
    /// when the fee could not be determined — shown as "unknown", never
    /// guessed.
    pub fee_estimate_sat: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RailsResponse {
    /// True when this checkout runs in test mode (signet): the checkout
    /// shows the TEST MODE banner. True when the server is in test mode or
    /// the invoice itself carries the test marker.
    pub test_mode: bool,
    /// Lightning rail details, or `null` when no LN invoice exists for
    /// this invoice (LN receive off, or the gateway was unavailable).
    pub lightning: Option<LightningRail>,
}

pub async fn rails(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RailsResponse>, AppError> {
    let id_for_lookup = id.clone();
    let (invoice, _store_name) = state
        .db
        .call(move |conn| invoice::get_public(conn, &id_for_lookup))
        .await?
        .ok_or(AppError::NotFound)?;

    let metadata: serde_json::Value = serde_json::from_str(&invoice.metadata_json)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));

    let test_mode = state.config.mode == "test"
        || metadata
            .get("test")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

    let invoice_id = invoice.id.clone();
    let bolt11 = state
        .db
        .call(move |conn| invoice::get_lightning_invoice(conn, &invoice_id))
        .await?;

    let lightning = bolt11.map(|bolt11| LightningRail {
        bolt11,
        fee_estimate_sat: metadata
            .get("lightning_fee_estimate_sat")
            .and_then(|value| value.as_i64()),
    });

    Ok(Json(RailsResponse {
        test_mode,
        lightning,
    }))
}
