use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::StoreAuth;
use crate::error::AppError;
use crate::invoice::{self, NewInvoice};
use crate::models::{Invoice, InvoiceStatus};
use crate::rates::fiat_to_sats;
use crate::routes::AppState;
use crate::util::{new_id, now_unix, to_rfc3339};

#[derive(Debug, Deserialize)]
pub struct FiatAmount {
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateInvoiceRequest {
    pub order_id: Option<String>,
    pub fiat: Option<FiatAmount>,
    pub sats: Option<i64>,
    pub expiry_minutes: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct InvoiceResponse {
    pub id: String,
    pub status: InvoiceStatus,
    pub amount_sats: i64,
    pub paid_sats: i64,
    pub fiat_currency: Option<String>,
    pub fiat_amount: Option<String>,
    pub rate: Option<String>,
    pub rate_source: Option<String>,
    pub rate_locked_at: Option<String>,
    pub checkout_url: String,
    pub ark_address: String,
    pub onchain_address: String,
    /// BOLT11 invoice from the ASP's Lightning gateway, when one exists.
    pub lightning_invoice: Option<String>,
    pub expires_at: String,
    pub created_at: String,
    pub paid_at: Option<String>,
    pub order_id: Option<String>,
    pub metadata: serde_json::Value,
}

fn to_response(
    invoice: &Invoice,
    base_url: &str,
    lightning_invoice: Option<String>,
) -> InvoiceResponse {
    let metadata = serde_json::from_str(&invoice.metadata_json)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));

    InvoiceResponse {
        id: invoice.id.clone(),
        status: invoice.status,
        amount_sats: invoice.amount_sats,
        paid_sats: invoice.paid_sats,
        fiat_currency: invoice.fiat_currency.clone(),
        fiat_amount: invoice.fiat_amount.clone(),
        rate: invoice.rate.clone(),
        rate_source: invoice.rate_source.clone(),
        rate_locked_at: invoice.rate_locked_at.map(to_rfc3339),
        checkout_url: format!(
            "{}/i/{}",
            base_url.trim_end_matches('/'),
            invoice.id
        ),
        ark_address: invoice.ark_address.clone(),
        onchain_address: invoice.onchain_address.clone(),
        lightning_invoice,
        expires_at: to_rfc3339(invoice.expires_at),
        created_at: to_rfc3339(invoice.created_at),
        paid_at: invoice.paid_at.map(to_rfc3339),
        order_id: invoice.order_id.clone(),
        metadata,
    }
}

pub async fn create(
    State(state): State<AppState>,
    auth: StoreAuth,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceResponse>), AppError> {
    if req.fiat.is_some() && req.sats.is_some() {
        return Err(AppError::BadRequest(
            "provide either `fiat` or `sats`, not both".into(),
        ));
    }

    let expiry_minutes = req
        .expiry_minutes
        .unwrap_or(state.config.default_expiry_minutes);

    if expiry_minutes < 1 || expiry_minutes > state.config.max_expiry_minutes {
        return Err(AppError::BadRequest(format!(
            "expiry_minutes must be between 1 and {}",
            state.config.max_expiry_minutes
        )));
    }

    let (amount_sats, fiat_currency, fiat_amount, rate, rate_source, rate_locked_at) =
        match (&req.fiat, req.sats) {
            (Some(fiat), None) => {
                let parsed: f64 = fiat.amount.trim().parse().map_err(|_| {
                    AppError::BadRequest("fiat.amount must be a decimal string".into())
                })?;
                let quote = state.rates.quote(&fiat.currency)?;
                let sats = fiat_to_sats(parsed, quote.fiat_per_btc)?;
                (
                    sats,
                    Some(quote.currency),
                    Some(fiat.amount.trim().to_string()),
                    Some(format!("{:.2}", quote.fiat_per_btc)),
                    Some(quote.source),
                    Some(quote.quoted_at),
                )
            }
            (None, Some(sats)) => {
                if sats <= 0 {
                    return Err(AppError::BadRequest(
                        "sats must be greater than zero".into(),
                    ));
                }
                (sats, None, None, None, None, None)
            }
            (None, None) => {
                return Err(AppError::BadRequest(
                    "provide either `fiat` or `sats`".into(),
                ))
            }
            (Some(_), Some(_)) => unreachable!("guarded above"),
        };

    let invoice_id = new_id("inv");
    let now = now_unix();

    // BARK INTEGRATION: Phase 1 derives these from the Bark wallet.
    let addresses = state
        .wallet
        .receive_addresses(&invoice_id)
        .map_err(|e| AppError::Internal(format!("wallet unavailable: {e}")))?;

    let metadata_json = req
        .metadata
        .as_ref()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "{}".to_string());

    let new_invoice = NewInvoice {
        id: invoice_id.clone(),
        store_id: auth.store.id.clone(),
        order_id: req.order_id.clone(),
        amount_sats,
        fiat_currency,
        fiat_amount,
        rate,
        rate_source,
        rate_locked_at,
        ark_address: addresses.ark_address,
        onchain_address: addresses.onchain_address,
        expires_at: now + expiry_minutes * 60,
        confs_required: 1,
        metadata_json,
    };

    let invoice = state
        .db
        .call(move |conn| invoice::insert(conn, &new_invoice))
        .await?;

    // Lightning receive (Phase 1): ask the wallet for an LN invoice through
    // the ASP's gateway. Best effort — when the gateway is unavailable the
    // checkout still works over Ark and on-chain.
    let lightning_invoice = if state.config.lightning_receive_enabled {
        match state
            .wallet
            .lightning_invoice(amount_sats, &format!("invoice {}", invoice.id))
        {
            Ok(ln) => {
                let bolt11 = ln.bolt11.clone();
                let identifier = ln.identifier.clone();
                let invoice_id = invoice.id.clone();
                let stored = state
                    .db
                    .call(move |conn| {
                        invoice::set_lightning_invoice(
                            conn,
                            &invoice_id,
                            &bolt11,
                            identifier.as_deref(),
                        )
                    })
                    .await;
                if let Err(err) = stored {
                    tracing::warn!(error = %err, "could not persist the lightning invoice");
                }
                Some(ln.bolt11)
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "lightning invoice unavailable; checkout continues with ark + on-chain"
                );
                None
            }
        }
    } else {
        None
    };

    let response = to_response(&invoice, &state.config.public_base_url, lightning_invoice);
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn get_one(
    State(state): State<AppState>,
    auth: StoreAuth,
    Path(id): Path<String>,
) -> Result<Json<InvoiceResponse>, AppError> {
    let store_id = auth.store.id.clone();
    let invoice = state
        .db
        .call(move |conn| {
            // This endpoint is the store's source of truth: an invoice past
            // its rate-lock deadline must read as `expired`, not `pending`,
            // even if the customer never opened the checkout page.
            invoice::expire_if_due(conn, &id)?;
            invoice::get(conn, &store_id, &id)
        })
        .await?
        .ok_or(AppError::NotFound)?;

    let invoice_id = invoice.id.clone();
    let lightning_invoice = state
        .db
        .call(move |conn| invoice::get_lightning_invoice(conn, &invoice_id))
        .await?;

    Ok(Json(to_response(
        &invoice,
        &state.config.public_base_url,
        lightning_invoice,
    )))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: StoreAuth,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<InvoiceResponse>>, AppError> {
    let status = match query.status.as_deref() {
        Some(raw) => Some(
            raw.parse::<InvoiceStatus>()
                .map_err(|_| AppError::BadRequest(format!("unknown status: {raw}")))?,
        ),
        None => None,
    };

    let store_id = auth.store.id.clone();
    let since = query.since;
    let limit = query.limit.unwrap_or(50);

    let invoices = state
        .db
        .call(move |conn| invoice::list(conn, &store_id, status, since, limit))
        .await?;

    let base_url = &state.config.public_base_url;
    Ok(Json(
        invoices
            .iter()
            .map(|invoice| to_response(invoice, base_url, None))
            .collect(),
    ))
}

pub async fn cancel(
    State(state): State<AppState>,
    auth: StoreAuth,
    Path(id): Path<String>,
) -> Result<Json<InvoiceResponse>, AppError> {
    let store_id = auth.store.id.clone();
    let invoice = state
        .db
        .call(move |conn| invoice::cancel(conn, &store_id, &id))
        .await?;

    Ok(Json(to_response(
        &invoice,
        &state.config.public_base_url,
        None,
    )))
}