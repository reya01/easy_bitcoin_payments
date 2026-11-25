//! The Invoices tab (docs/dev/INVOICES.md): invoices the owner makes by hand
//! and sends to a customer by copy and share. Owner-only: every route needs a
//! signed-in admin session (and its CSRF token for changes); a shop's API key
//! is never enough. In testing, and off until the owner turns it on.
//!
//! The note and the customer name are never written to the log.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::invoice::{self, NewInvoice};
use crate::manual_invoice::{self, ManualInvoice};
use crate::models::InvoiceStatus;
use crate::rates::fiat_to_sats;
use crate::routes::invoices::{csv_cell, fetch_lightning_fee_estimate, FiatAmount};
use crate::routes::AppState;
use crate::util::{new_id, now_unix, to_rfc3339};

/// The Ark server a payer's wallet must use, named in the payment request.
/// Only known for certain on a real-money server.
fn ark_server(config: &crate::config::Config) -> Option<&'static str> {
    (config.mode == "live").then_some("ark.second.tech")
}

#[derive(Debug, Serialize)]
pub struct ManualInvoiceResponse {
    pub id: String,
    pub invoice_number: Option<String>,
    pub customer_name: Option<String>,
    pub note: Option<String>,
    pub status: InvoiceStatus,
    pub amount_sats: i64,
    pub paid_sats: i64,
    pub fiat_currency: Option<String>,
    pub fiat_amount: Option<String>,
    pub ark_address: String,
    pub lightning_invoice: Option<String>,
    /// When the Lightning invoice stops working, read from the invoice itself.
    pub lightning_expires_at: Option<String>,
    /// The Lightning invoice has run out long enough ago that a fresh one can
    /// be made (see `manual_invoice::FRESH_LIGHTNING_GRACE_SECS`).
    pub can_refresh_lightning: bool,
    pub lightning_fee_estimate_sat: Option<i64>,
    pub expires_at: String,
    pub created_at: String,
    pub paid_at: Option<String>,
    pub store_name: String,
    pub ark_server: Option<String>,
    pub test_mode: bool,
}

fn lightning_expiry(m: &ManualInvoice) -> Option<i64> {
    m.bolt11
        .as_deref()
        .and_then(manual_invoice::bolt11_expires_at)
}

fn can_refresh(m: &ManualInvoice, now: i64, lightning_on: bool) -> bool {
    lightning_on
        && m.invoice.status == InvoiceStatus::Pending
        && m.invoice.expires_at > now
        && match lightning_expiry(m) {
            Some(t) => now >= t + manual_invoice::FRESH_LIGHTNING_GRACE_SECS,
            // No Lightning invoice yet (the wallet could not make one): try again.
            None => true,
        }
}

fn to_response(m: &ManualInvoice, state: &AppState, store_name: &str) -> ManualInvoiceResponse {
    let inv = &m.invoice;
    let metadata: serde_json::Value = serde_json::from_str(&inv.metadata_json)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
    ManualInvoiceResponse {
        id: inv.id.clone(),
        invoice_number: m.invoice_number.clone(),
        customer_name: m.customer_name.clone(),
        note: m.note.clone(),
        status: inv.status,
        amount_sats: inv.amount_sats,
        paid_sats: inv.paid_sats,
        fiat_currency: inv.fiat_currency.clone(),
        fiat_amount: inv.fiat_amount.clone(),
        ark_address: inv.ark_address.clone(),
        lightning_invoice: m.bolt11.clone(),
        lightning_expires_at: lightning_expiry(m).map(to_rfc3339),
        can_refresh_lightning: can_refresh(m, now_unix(), state.config.lightning_receive_enabled),
        lightning_fee_estimate_sat: metadata
            .get("lightning_fee_estimate_sat")
            .and_then(|v| v.as_i64()),
        expires_at: to_rfc3339(inv.expires_at),
        created_at: to_rfc3339(inv.created_at),
        paid_at: inv.paid_at.map(to_rfc3339),
        store_name: store_name.to_string(),
        ark_server: ark_server(&state.config).map(str::to_string),
        test_mode: state.config.mode == "test",
    }
}

async fn require_enabled(state: &AppState) -> Result<(), AppError> {
    if state.db.call(|c| manual_invoice::is_enabled(c)).await? {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "Invoices are turned off. Turn them on in Settings, Invoices (in testing).".into(),
        ))
    }
}

/// `GET /api/v1/admin/invoices/settings` — is the tab on, the next default
/// number, and customer names used before (for suggestions).
pub async fn settings(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<serde_json::Value>, AppError> {
    let store_id = auth.store.id.clone();
    let (enabled, next, names) = state
        .db
        .call(move |conn| {
            Ok((
                manual_invoice::is_enabled(conn)?,
                crate::invoice_ident::next_invoice_number(conn)?,
                crate::invoice_ident::customer_name_suggestions(conn, &store_id, 50)?,
            ))
        })
        .await?;
    Ok(Json(serde_json::json!({
        "enabled": enabled,
        "next_number": next,
        "customer_names": names,
        "store_name": auth.store.name,
        "open_hours": manual_invoice::OPEN_SECS / 3600,
        "lightning_hours": manual_invoice::LIGHTNING_SECS / 3600,
        "default_currency": state.config.default_currency,
        // Money the amount can be given in: only currencies with a recent price.
        "currencies": state.rates.available_currencies(),
        "ark_server": ark_server(&state.config),
        "test_mode": state.config.mode == "test",
    })))
}

#[derive(Debug, Deserialize)]
pub struct EnabledRequest {
    pub enabled: bool,
}

/// `POST /api/v1/admin/invoices/enabled` — turn the tab on or off. Turning it
/// off hides the tab; invoices already sent still get paid and show up in
/// the payments list.
pub async fn set_enabled(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Json(req): Json<EnabledRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store_id = auth.store.id.clone();
    let on = req.enabled;
    state
        .db
        .call(move |conn| {
            manual_invoice::set_enabled(conn, on)?;
            crate::audit::record(
                conn,
                &store_id,
                if on {
                    "invoices.enabled"
                } else {
                    "invoices.disabled"
                },
                None,
            )
        })
        .await?;
    Ok(Json(serde_json::json!({ "enabled": on })))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
    pub limit: Option<i64>,
}

/// `GET /api/v1/admin/invoices` — newest first; `q` searches the number and
/// the customer name. Readable even while the tab is off.
pub async fn list(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<ManualInvoiceResponse>>, AppError> {
    let store_id = auth.store.id.clone();
    let limit = query.limit.unwrap_or(50);
    let rows = state
        .db
        .call(move |conn| manual_invoice::list(conn, &store_id, query.q.as_deref(), limit))
        .await?;
    Ok(Json(
        rows.iter()
            .map(|m| to_response(m, &state, &auth.store.name))
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub sats: Option<i64>,
    pub fiat: Option<FiatAmount>,
    pub note: Option<String>,
    pub invoice_number: Option<String>,
    pub customer_name: Option<String>,
}

/// Ask the wallet for a Lightning invoice for this invoice and store it.
/// Best effort, like checkout: without a known Ark server fee, or if the
/// wallet says no, the invoice is Ark only (and can try again later).
async fn issue_lightning(
    state: &AppState,
    invoice_id: &str,
    amount_sats: i64,
    label: &str,
    seconds_left: i64,
) -> Result<(), AppError> {
    if !state.config.lightning_receive_enabled {
        return Ok(());
    }
    let Some(fee) = fetch_lightning_fee_estimate(&state.config, amount_sats).await else {
        tracing::warn!("lightning receive fee unknown; this invoice offers Ark only for now");
        return Ok(());
    };
    let wallet = state.wallet.clone();
    let description = label.to_string();
    let ln_amount = amount_sats + fee.max(0);
    // EBP asks for as long as the invoice is open; the Ark server shortens it
    // to its own limit, and the real time is read back from the invoice.
    let expiry_secs = seconds_left.max(60);
    let result = tokio::task::spawn_blocking(move || {
        wallet.lightning_invoice(ln_amount, &description, expiry_secs)
    })
    .await
    .unwrap_or_else(|e| {
        Err(crate::wallet::WalletError::Unavailable(format!(
            "wallet task failed: {e}"
        )))
    });
    match result {
        Ok(ln) => {
            let id = invoice_id.to_string();
            state
                .db
                .call(move |conn| {
                    invoice::set_lightning_invoice(conn, &id, &ln.bolt11, ln.identifier.as_deref())?;
                    // Remember the fee shown, like checkout does.
                    conn.execute(
                        "UPDATE invoices SET metadata_json = json_set(metadata_json, '$.lightning_fee_estimate_sat', ?2) WHERE id = ?1 AND json_valid(metadata_json)",
                        rusqlite::params![id, fee],
                    )?;
                    Ok(())
                })
                .await
        }
        Err(err) => {
            tracing::warn!(error = %err, "lightning invoice unavailable; this invoice offers Ark only for now");
            Ok(())
        }
    }
}

/// `POST /api/v1/admin/invoices` — make an invoice.
pub async fn create(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Json(req): Json<CreateRequest>,
) -> Result<(StatusCode, Json<ManualInvoiceResponse>), AppError> {
    require_enabled(&state).await?;
    if state.db.call(|c| crate::shopkey::is_paused(c)).await? {
        return Err(AppError::Conflict(
            "New payments are paused. Turn payments back on in Settings, Pause new payments, first.".into(),
        ));
    }
    if !crate::ratelimit::allow_keyed(
        state.config.invoice_rate_limit_per_min,
        &format!("manual:{}", auth.store.id),
        now_unix(),
    ) {
        return Err(AppError::TooManyRequests(
            "Too many invoices in a minute. Please wait a moment and try again.".into(),
        ));
    }

    let (amount_sats, fiat_currency, fiat_amount, rate, rate_source, rate_locked_at) =
        match (&req.fiat, req.sats) {
            (Some(_), Some(_)) => {
                return Err(AppError::BadRequest(
                    "Give the amount in sats or in money, not both.".into(),
                ))
            }
            (None, None) => return Err(AppError::BadRequest("Enter an amount.".into())),
            (None, Some(sats)) => {
                if sats <= 0 || sats > invoice::MAX_INVOICE_SATS {
                    return Err(AppError::BadRequest(
                        "Enter an amount in sats greater than zero.".into(),
                    ));
                }
                (sats, None, None, None, None, None)
            }
            (Some(fiat), None) => {
                let parsed: f64 = fiat.amount.trim().parse().map_err(|_| {
                    AppError::BadRequest("Enter the amount as a number, like 25 or 25.50.".into())
                })?;
                if !(parsed.is_finite() && parsed > 0.0) {
                    return Err(AppError::BadRequest(
                        "Enter an amount greater than zero.".into(),
                    ));
                }
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
        };
    if amount_sats > state.config.max_invoice_sats {
        return Err(AppError::BadRequest(format!(
            "This is larger than this server accepts ({} sats).",
            state.config.max_invoice_sats
        )));
    }

    let note = manual_invoice::clean_note(req.note.as_deref().unwrap_or(""))?;
    let typed_number =
        crate::invoice_ident::clean_invoice_number(req.invoice_number.as_deref().unwrap_or(""))?;
    let customer_name =
        crate::invoice_ident::clean_customer_name(req.customer_name.as_deref().unwrap_or(""))?
            .unwrap_or_default();

    // The number: the typed one if free, otherwise the next one counting up.
    let number = state
        .db
        .call(move |conn| match typed_number {
            Some(n) => {
                if manual_invoice::number_taken(conn, &n)? {
                    Err(AppError::Conflict(
                        "That invoice number is already used by another invoice. Choose a different one.".into(),
                    ))
                } else {
                    Ok(n)
                }
            }
            None => crate::invoice_ident::next_invoice_number(conn),
        })
        .await?;

    let invoice_id = new_id("inv");
    let now = now_unix();
    let seconds_open = manual_invoice::OPEN_SECS;

    // A fresh Ark address that no open payment request is using.
    let fresh_address = || {
        let wallet = state.wallet.clone();
        let id = invoice_id.clone();
        async move {
            tokio::task::spawn_blocking(move || wallet.receive_addresses(&id))
                .await
                .map_err(|e| AppError::Internal(format!("wallet task failed: {e}")))?
                .map_err(|e| AppError::Internal(format!("wallet unavailable: {e}")))
        }
    };
    let mut addresses = fresh_address().await?;
    for attempt in 0..2 {
        let ark = addresses.ark_address.clone();
        let taken = state
            .db
            .call(move |conn| invoice::ark_address_in_use(conn, &ark))
            .await?;
        if !taken {
            break;
        }
        if attempt == 1 {
            return Err(AppError::Unavailable(
                "The wallet could not give a new payment address just now. Please try again in a moment.".into(),
            ));
        }
        tracing::warn!("the wallet repeated a payment address that is still in use; asking again");
        addresses = fresh_address().await?;
    }

    let mut metadata = serde_json::Map::new();
    if state.config.mode == "test" {
        metadata.insert("test".into(), serde_json::Value::Bool(true));
    }
    let new_invoice = NewInvoice {
        id: invoice_id.clone(),
        store_id: auth.store.id.clone(),
        order_id: None,
        amount_sats,
        fiat_currency,
        fiat_amount,
        rate,
        rate_source,
        rate_locked_at,
        ark_address: addresses.ark_address,
        expires_at: now + seconds_open,
        confs_required: 1,
        metadata_json: serde_json::Value::Object(metadata).to_string(),
    };
    let store_id = auth.store.id.clone();
    let number_for_db = number.clone();
    state
        .db
        .call(move |conn| {
            invoice::insert_with_cap(
                conn,
                &new_invoice,
                invoice::MAX_OPEN_INVOICES,
                invoice::MAX_OPEN_INVOICES_PER_STORE,
            )?;
            manual_invoice::finish_new(
                conn,
                &store_id,
                &new_invoice.id,
                note.as_deref(),
                &number_for_db,
                &customer_name,
            )
        })
        .await?;

    issue_lightning(
        &state,
        &invoice_id,
        amount_sats,
        &format!("Invoice {number}"),
        manual_invoice::LIGHTNING_SECS,
    )
    .await?;

    let store_id = auth.store.id.clone();
    let id = invoice_id.clone();
    let made = state
        .db
        .call(move |conn| manual_invoice::get(conn, &store_id, &id))
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((
        StatusCode::CREATED,
        Json(to_response(&made, &state, &auth.store.name)),
    ))
}

/// `POST /api/v1/admin/invoices/{id}/lightning` — a fresh Lightning invoice
/// once the old one has run out (the Ark address stays the same).
pub async fn fresh_lightning(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<String>,
) -> Result<Json<ManualInvoiceResponse>, AppError> {
    require_enabled(&state).await?;
    let store_id = auth.store.id.clone();
    let lookup = id.clone();
    let m = state
        .db
        .call(move |conn| manual_invoice::get(conn, &store_id, &lookup))
        .await?
        .ok_or(AppError::NotFound)?;
    let now = now_unix();
    if !can_refresh(&m, now, state.config.lightning_receive_enabled) {
        return Err(AppError::Conflict(
            if !state.config.lightning_receive_enabled {
                "Lightning is turned off on this server; the Ark option still works.".to_string()
            } else if m.invoice.status != InvoiceStatus::Pending || m.invoice.expires_at <= now {
                "This invoice is no longer open, so it gets no new Lightning invoice.".to_string()
            } else {
                "The current Lightning invoice still works (or ran out only minutes ago). A fresh one can be made about 10 minutes after it runs out.".to_string()
            },
        ));
    }
    let label = format!("Invoice {}", m.invoice_number.as_deref().unwrap_or(""));
    issue_lightning(
        &state,
        &m.invoice.id,
        m.invoice.amount_sats,
        label.trim(),
        (m.invoice.expires_at - now).min(manual_invoice::LIGHTNING_SECS),
    )
    .await?;
    let store_id = auth.store.id.clone();
    let m = state
        .db
        .call(move |conn| manual_invoice::get(conn, &store_id, &id))
        .await?
        .ok_or(AppError::NotFound)?;
    if m.bolt11.is_none() || !lightning_expiry(&m).is_some_and(|t| t > now) {
        return Err(AppError::Unavailable(
            "The wallet could not make a Lightning invoice just now. The Ark option still works; try again in a few minutes.".into(),
        ));
    }
    Ok(Json(to_response(&m, &state, &auth.store.name)))
}

/// `POST /api/v1/admin/invoices/{id}/cancel`.
pub async fn cancel(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<String>,
) -> Result<Json<ManualInvoiceResponse>, AppError> {
    let store_id = auth.store.id.clone();
    let m = state
        .db
        .call(move |conn| {
            if manual_invoice::get(conn, &store_id, &id)?.is_none() {
                return Err(AppError::NotFound);
            }
            invoice::cancel(conn, &store_id, &id)?;
            manual_invoice::get(conn, &store_id, &id)?.ok_or(AppError::NotFound)
        })
        .await?;
    Ok(Json(to_response(&m, &state, &auth.store.name)))
}

/// `GET /api/v1/admin/invoices.csv` — every invoice from the Invoices tab,
/// oldest first. The number and customer name come first.
pub async fn export_csv(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<axum::response::Response, AppError> {
    use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
    use axum::response::IntoResponse;

    let store_id = auth.store.id.clone();
    let rows = state
        .db
        .call(move |conn| manual_invoice::export_all(conn, &store_id))
        .await?;
    let mut out = String::from(
        "\u{feff}invoice_number,customer_name,created_at,paid_at,status,amount_sats,paid_sats,fiat_currency,fiat_amount,note\r\n",
    );
    for m in &rows {
        let inv = &m.invoice;
        let cells = [
            csv_cell(m.invoice_number.as_deref().unwrap_or("")),
            csv_cell(m.customer_name.as_deref().unwrap_or("")),
            csv_cell(&to_rfc3339(inv.created_at)),
            csv_cell(&inv.paid_at.map(to_rfc3339).unwrap_or_default()),
            csv_cell(inv.status.as_str()),
            inv.amount_sats.to_string(),
            inv.paid_sats.to_string(),
            csv_cell(inv.fiat_currency.as_deref().unwrap_or("")),
            csv_cell(inv.fiat_amount.as_deref().unwrap_or("")),
            csv_cell(m.note.as_deref().unwrap_or("")),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    Ok((
        [
            (CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                CONTENT_DISPOSITION,
                "attachment; filename=\"ebp-invoices.csv\"",
            ),
            (CACHE_CONTROL, "no-store"),
        ],
        out,
    )
        .into_response())
}
