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
    /// Estimated Lightning gateway fee for this invoice, in sats, fetched
    /// from the ASP's live fee table at creation time. `null` when LN
    /// receive is off or the fee could not be determined — never guessed.
    /// The fee is the PAYER's: the customer's wallet sends amount + fee
    /// (drill 2026-11-01: 5,020 sent for a 5,000-sat invoice,
    /// `offchain_fee_sat: 20` on the payer, 0 on the payee), so the payee
    /// receives the full invoice amount — never deducted from the payout.
    pub lightning_fee_estimate_sat: Option<i64>,
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
        checkout_url: format!("{}/i/{}", base_url.trim_end_matches('/'), invoice.id),
        ark_address: invoice.ark_address.clone(),
        onchain_address: invoice.onchain_address.clone(),
        lightning_invoice,
        lightning_fee_estimate_sat: metadata
            .get("lightning_fee_estimate_sat")
            .and_then(|value| value.as_i64()),
        expires_at: to_rfc3339(invoice.expires_at),
        created_at: to_rfc3339(invoice.created_at),
        paid_at: invoice.paid_at.map(to_rfc3339),
        order_id: invoice.order_id.clone(),
        metadata,
    }
}

/// Lightning receive fee table on the ASP gateway. GET.
///
/// VERIFY: path + response shape — `/fees/lightning/receive` is not in
/// the pinned `docs/barkd-openapi.json` endpoint list, so the path is
/// verified against a live barkd before this marker comes off. The
/// response is parsed defensively (see [`parse_lightning_fee_estimate`]):
/// an unrecognised shape yields `None` (shown as "fee unknown"), never a
/// guess. This lives here rather than on `BarkdClient` so the invoice
/// route stays self-contained until the async `WalletAdapter` migration
/// (plan task 4) gives it a natural home.
const PATH_LN_RECEIVE_FEES: &str = "/fees/lightning/receive";

/// Ask barkd for the ASP gateway's Lightning receive fee for `amount_sats`.
/// Best effort: any transport or HTTP failure is logged (without secrets)
/// and yields `None` — the checkout stays usable over Ark and on-chain.
async fn fetch_lightning_fee_estimate(
    config: &crate::config::Config,
    amount_sats: i64,
) -> Option<i64> {
    let token = config.barkd_token.as_deref()?;

    let url = format!(
        "{}{}",
        config.barkd_url.trim_end_matches('/'),
        PATH_LN_RECEIVE_FEES
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
    header.set_sensitive(true);

    let response = client
        .get(&url)
        .query(&[("amount_sat", amount_sats.to_string())])
        .header(reqwest::header::AUTHORIZATION, header)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "lightning fee table unavailable");
        return None;
    }

    match response.json::<serde_json::Value>().await {
        Ok(body) => parse_lightning_fee_estimate(&body, amount_sats),
        Err(err) => {
            tracing::warn!(error = %err, "lightning fee table was not valid JSON");
            None
        }
    }
}

/// Pull a sats fee estimate for `amount_sats` out of the fee-table JSON.
/// Recognised shapes (VERIFY against a live barkd):
///   {"fee_sat": 12} | {"fee_sats": 12} | {"fee": 12}
///   {"fees": {"receive": 12}} | {"fees": {"fee_sat": 12}}
///   {"fee_percent": 0.5}  → ceil(amount × percent / 100)
/// Anything else → `None`.
fn parse_lightning_fee_estimate(body: &serde_json::Value, amount_sats: i64) -> Option<i64> {
    fn as_sats(value: &serde_json::Value) -> Option<i64> {
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|fee| fee as i64))
            .map(|sats| sats.max(0))
    }

    for key in ["fee_sat", "fee_sats", "fee"] {
        if let Some(fee) = body.get(key).and_then(as_sats) {
            return Some(fee);
        }
    }

    if let Some(fees) = body.get("fees") {
        for key in ["receive", "fee_sat", "fee_sats"] {
            if let Some(fee) = fees.get(key).and_then(as_sats) {
                return Some(fee);
            }
        }
    }

    if let Some(percent) = body.get("fee_percent").and_then(|value| value.as_f64()) {
        if percent.is_finite() && percent >= 0.0 {
            let estimate = (amount_sats as f64 * percent / 100.0).ceil();
            if estimate.is_finite() && estimate >= 0.0 && estimate <= i64::MAX as f64 {
                return Some(estimate as i64);
            }
        }
    }

    None
}

#[cfg(test)]
mod lightning_fee_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn fee_estimate_parses_known_shapes() {
        let cases = [
            (r#"{"fee_sat": 12}"#, Some(12i64)),
            (r#"{"fee_sats": 12}"#, Some(12)),
            (r#"{"fee": 12}"#, Some(12)),
            (r#"{"fees": {"receive": 12}}"#, Some(12)),
            (r#"{"fees": {"fee_sat": 12}}"#, Some(12)),
            (r#"{"fee_percent": 0.5}"#, Some(250)), // 0.5% of 50_000
        ];
        for (body, expected) in cases {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                parse_lightning_fee_estimate(&parsed, 50_000),
                expected,
                "body: {body}"
            );
        }
    }

    #[test]
    fn fee_estimate_is_none_for_unknown_shapes() {
        for body in [
            r#"{}"#,
            r#"{"min": 1000}"#,
            r#"{"fee_percent": -1}"#,
            r#"{"fee": "cheap"}"#,
        ] {
            let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                parse_lightning_fee_estimate(&parsed, 50_000),
                None,
                "body: {body}"
            );
        }
    }

    #[test]
    fn fee_estimate_is_fetched_from_mock_barkd_over_http() {
        // Mock barkd: one GET, one canned fee table, then done.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut buf = [0u8; 4096];
            while !request.contains("\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            assert!(
                request.starts_with("GET /api/v1/fees/lightning/receive"),
                "unexpected request: {request}"
            );
            let body = r#"{"fee_sat": 12}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });

        let config = crate::config::Config {
            barkd_url: format!("http://127.0.0.1:{port}/api/v1"),
            barkd_token: Some("test-token".to_string()),
            ..crate::config::Config::default()
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let fee = runtime.block_on(fetch_lightning_fee_estimate(&config, 50_000));

        assert_eq!(fee, Some(12));
        server.join().unwrap();
    }

    #[test]
    fn fee_estimate_is_none_without_a_token() {
        let config = crate::config::Config::default();
        assert!(config.barkd_token.is_none());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime.block_on(fetch_lightning_fee_estimate(&config, 50_000)),
            None
        );
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
                // Upper bound = the entire bitcoin money supply in sats;
                // anything above is a typo, not an invoice.
                if sats > crate::invoice::MAX_INVOICE_SATS {
                    return Err(AppError::BadRequest(
                        "sats is out of the supported range".into(),
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

    // IDEMPOTENCY (beta hardening): a retry for the same `order_id` with the
    // same amount returns the existing open invoice (200 OK) instead of
    // minting a new one. "Same amount" = same fiat amount+currency for fiat
    // requests (the sats figure drifts with the rate), same sats otherwise.
    // A changed amount creates a new invoice; the old one simply expires.
    if let Some(order_id) = req.order_id.clone().filter(|o| !o.trim().is_empty()) {
        let store_id = auth.store.id.clone();
        let existing = state
            .db
            .call(move |conn| invoice::find_open_by_order_id(conn, &store_id, &order_id))
            .await?;
        if let Some(existing) = existing {
            let same_amount = match (&req.fiat, req.sats) {
                (Some(fiat), None) => {
                    existing.fiat_currency.as_deref().map(str::to_uppercase)
                        == Some(fiat.currency.trim().to_uppercase())
                        && existing.fiat_amount.as_deref() == Some(fiat.amount.trim())
                }
                _ => existing.fiat_currency.is_none() && existing.amount_sats == amount_sats,
            };
            if same_amount {
                let invoice_id = existing.id.clone();
                let lightning_invoice = state
                    .db
                    .call(move |conn| invoice::get_lightning_invoice(conn, &invoice_id))
                    .await?;
                let response =
                    to_response(&existing, &state.config.public_base_url, lightning_invoice);
                return Ok((StatusCode::OK, Json(response)));
            }
        }
    }

    // Lightning gateway fee (Phase 1 close-out): fetched from the ASP's
    // live fee table at invoice time when LN receive is on. Best effort —
    // an unavailable or unrecognised fee is reported as `null`, never
    // guessed, and never blocks checkout.
    let lightning_fee_estimate_sat = if state.config.lightning_receive_enabled {
        fetch_lightning_fee_estimate(&state.config, amount_sats).await
    } else {
        None
    };

    let invoice_id = new_id("inv");
    let now = now_unix();

    // BARK INTEGRATION: Phase 1 derives these from the Bark wallet.
    let addresses = state
        .wallet
        .receive_addresses(&invoice_id)
        .map_err(|e| AppError::Internal(format!("wallet unavailable: {e}")))?;

    // Metadata carries the store's own key/value data plus two internal
    // markers: `test` (set in test mode; test invoices are excluded from
    // the accounting export — see invoice::is_test_invoice) and
    // `lightning_fee_estimate_sat` (the LN fee shown at checkout). Both
    // ride in metadata_json so every reader of the invoice — API,
    // checkout, export — sees the same flags. A store that sends
    // non-object metadata keeps it verbatim and the markers are skipped.
    let mut metadata = req
        .metadata
        .clone()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(fields) = metadata.as_object_mut() {
        // Reserved keys are server-set: a merchant could otherwise fake
        // `test: true` (their invoice silently vanishes from the accounting
        // export) or inject a bogus `lightning_fee_estimate_sat` that
        // checkout would display as authoritative.
        fields.remove("test");
        fields.remove("lightning_fee_estimate_sat");
        if state.config.mode == "test" {
            fields.insert("test".to_string(), serde_json::Value::Bool(true));
        }
        if let Some(fee) = lightning_fee_estimate_sat {
            fields.insert(
                "lightning_fee_estimate_sat".to_string(),
                serde_json::Value::from(fee),
            );
        }
    }
    let metadata_json = metadata.to_string();

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
                match stored {
                    Ok(()) => Some(ln.bolt11),
                    Err(err) => {
                        // If the BOLT11 is not persisted, the watcher cannot
                        // attribute an LN payment back to this invoice. Do not
                        // show a Lightning option the server cannot credit.
                        tracing::warn!(
                            error = %err,
                            "could not persist the lightning invoice; hiding the LN rail for this invoice"
                        );
                        None
                    }
                }
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
