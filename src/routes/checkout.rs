use axum::extract::{Path, State};
use axum::response::Html;
use axum::Json;
use serde::Serialize;

use crate::error::AppError;
use crate::invoice;
use crate::models::InvoiceStatus;
use crate::routes::AppState;
use crate::util::{now_unix, to_rfc3339};

const CHECKOUT_HTML: &str = include_str!("../../assets/checkout.html");

/// Hosted checkout page. Static shell; the page polls `/i/{id}/status`.
pub async fn page() -> Html<&'static str> {
    Html(CHECKOUT_HTML)
}

#[derive(Debug, Serialize)]
pub struct CheckoutStatus {
    pub id: String,
    pub status: InvoiceStatus,
    pub is_final: bool,
    pub amount_sats: i64,
    pub paid_sats: i64,
    pub fiat_currency: Option<String>,
    pub fiat_amount: Option<String>,
    pub ark_address: String,
    pub onchain_address: String,
    pub expires_at: String,
    pub expires_in_seconds: i64,
    pub store_name: String,
    pub policy_url: Option<String>,
    pub accent_color: String,
}

pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<CheckoutStatus>, AppError> {
    let invoice_id = id.clone();
    let invoice = state
        .db
        .call(move |conn| invoice::expire_if_due(conn, &invoice_id))
        .await?
        .ok_or(AppError::NotFound)?;

    let store_id = invoice.store_id.clone();
    let store = state
        .db
        .call(move |conn| crate::store::get(conn, &store_id))
        .await?
        .ok_or(AppError::NotFound)?;

    let now = now_unix();

    Ok(Json(CheckoutStatus {
        id: invoice.id.clone(),
        status: invoice.status,
        is_final: invoice.status.is_terminal(),
        amount_sats: invoice.amount_sats,
        paid_sats: invoice.paid_sats,
        fiat_currency: invoice.fiat_currency.clone(),
        fiat_amount: invoice.fiat_amount.clone(),
        ark_address: invoice.ark_address.clone(),
        onchain_address: invoice.onchain_address.clone(),
        expires_at: to_rfc3339(invoice.expires_at),
        expires_in_seconds: (invoice.expires_at - now).max(0),
        store_name: store.name,
        policy_url: state.config.checkout_policy_url.clone(),
        accent_color: state.config.checkout_accent_color.clone(),
    }))
}