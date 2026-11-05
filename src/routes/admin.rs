//! Operator status endpoint (beta hardening).
//!
//! `GET /api/v1/admin/status` — authenticated with `EBP_ADMIN_TOKEN`
//! (`Authorization: Bearer <admin token>`), NOT a store API key. When no admin
//! token is configured the endpoint does not exist (404). It answers the
//! questions an operator has that the public `/health` deliberately must not:
//! how much is in the till, is the on-chain exit reserve funded, is anything
//! waiting for the merchant (late payments, failed webhooks).
//!
//! `?funding_address=1` additionally asks barkd for a FRESH on-chain address
//! to top up the exit reserve. It is opt-in because every call consumes an
//! address from the wallet.
//!
//! Wallet figures come from barkd on a best-effort basis: any failure is
//! reported in `wallet.error` and the DB-derived counters are still returned.

use axum::extract::{Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::AppState;
use crate::util::to_rfc3339;
use crate::wallet::barkd::BarkdClient;
use crate::wallet::settlement::exit_reserve;

#[derive(Debug, Deserialize)]
pub struct StatusQuery {
    #[serde(default)]
    pub funding_address: Option<String>,
}

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    // A signed-in browser session may read the status (GET only).
    if let Some(token) = crate::admin_auth::token_from_headers(headers) {
        let live = state
            .db
            .call(move |c| crate::admin_auth::lookup_session(c, &token))
            .await?;
        if live.is_some() {
            return Ok(());
        }
    }
    // Endpoint is invisible unless an admin token is configured.
    let Some(expected) = state
        .config
        .admin_token
        .as_deref()
        .filter(|t| !t.is_empty())
    else {
        return Err(AppError::NotFound);
    };
    let supplied = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    if crate::auth::ct_eq(expected, supplied) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<StatusQuery>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &headers).await?;

    let (counters, recent_settlements) = state
        .db
        .call(|conn| {
            let count = |sql: &str| -> Result<i64, AppError> {
                Ok(conn.query_row(sql, [], |r| r.get::<_, i64>(0))?)
            };
            let counters = json!({
                "invoices_pending": count("SELECT COUNT(*) FROM invoices WHERE status = 'pending'")?,
                "unmatched_payments_open":
                    count("SELECT COUNT(*) FROM unmatched_payments WHERE resolved_at IS NULL")?,
                "webhooks_pending":
                    count("SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'pending'")?,
                "webhooks_failed":
                    count("SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'failed'")?,
            });
            let mut stmt = conn.prepare(
                "SELECT kind, amount_sats, destination, status, detail, created_at
                 FROM settlements ORDER BY created_at DESC LIMIT 10",
            )?;
            let rows = stmt
                .query_map(params![], |row| {
                    Ok(json!({
                        "kind": row.get::<_, String>(0)?,
                        "amount_sats": row.get::<_, i64>(1)?,
                        "destination": row.get::<_, Option<String>>(2)?,
                        "status": row.get::<_, String>(3)?,
                        "detail": row.get::<_, Option<String>>(4)?,
                        "created_at": to_rfc3339(row.get::<_, i64>(5)?),
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((counters, rows))
        })
        .await?;

    // Wallet / exit-reserve figures straight from barkd (best effort).
    let want_address = query.funding_address.as_deref() == Some("1");
    let wallet = match BarkdClient::from_app_config(&state.config) {
        Err(err) => json!({ "error": err.to_string() }),
        Ok(client) => {
            let mut out = serde_json::Map::new();
            let mut errors: Vec<String> = Vec::new();
            match client.balances().await {
                Ok(b) => {
                    out.insert("offchain_sats".into(), json!(b.offchain_sats));
                    out.insert("onchain_sats".into(), json!(b.onchain_sats));
                    out.insert(
                        "hot_wallet_cap_sats".into(),
                        json!(state.config.hot_wallet_cap_sats),
                    );
                }
                Err(e) => errors.push(format!("balances: {e}")),
            }
            match (client.vtxos().await, client.onchain_balance().await) {
                (Ok(vtxos), Ok(onchain)) => {
                    let reserve = exit_reserve(onchain.confirmed_sats, vtxos.spendable.len());
                    out.insert(
                        "exit_reserve".into(),
                        json!({
                            "ok": reserve.ok,
                            "confirmed_onchain_sats": reserve.onchain_sats,
                            "required_sats": reserve.required_sats,
                            "shortfall_sats": reserve.shortfall_sats,
                            "spendable_vtxos": reserve.vtxo_count,
                        }),
                    );
                }
                (Err(e), _) | (_, Err(e)) => errors.push(format!("exit reserve: {e}")),
            }
            if want_address {
                match client.onchain_address().await {
                    Ok(addr) => {
                        out.insert("reserve_funding_address".into(), json!(addr));
                    }
                    Err(e) => errors.push(format!("funding address: {e}")),
                }
            }
            if !errors.is_empty() {
                out.insert("error".into(), json!(errors.join("; ")));
            }
            Value::Object(out)
        }
    };

    Ok(Json(json!({
        "mode": state.config.mode,
        "onchain_rail_enabled": state.config.onchain_rail_enabled,
        "version": crate::VERSION,
        "counters": counters,
        "recent_settlements": recent_settlements,
        "wallet": wallet,
    })))
}
