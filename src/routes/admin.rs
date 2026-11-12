//! Operator status endpoint (beta hardening).
//!
//! `GET /api/v1/admin/status` — authenticated with `EBP_ADMIN_TOKEN`
//! (`Authorization: Bearer <admin token>`), NOT a store API key. When no admin
//! token is configured the endpoint does not exist (404). It answers the
//! questions an operator has that the public `/health` deliberately must not:
//! how much is in the till, is anything waiting for the merchant (late
//! payments, failed webhooks).
//!
//! Wallet figures come from barkd on a best-effort basis: any failure is
//! reported in `wallet.error` and the DB-derived counters are still returned.

use axum::extract::State;
use axum::http::header::AUTHORIZATION;
use axum::http::HeaderMap;
use axum::Json;
use rusqlite::params;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::AppState;
use crate::util::to_rfc3339;
use crate::wallet::barkd::BarkdClient;

async fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), AppError> {
    // A signed-in browser session may read the status (GET only).
    if let Some(token) = crate::admin_auth::token_from_headers(
        headers,
        crate::admin_auth::is_secure(&state.config.public_base_url),
    ) {
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
) -> Result<Json<Value>, AppError> {
    authorize(&state, &headers).await?;

    let (counters, recent_settlements, webhook_configured) = state
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
                 FROM settlements ORDER BY created_at DESC LIMIT 40",
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
            let configured = count(
                "SELECT COUNT(*) FROM stores WHERE webhook_url IS NOT NULL AND webhook_url <> ''",
            )? > 0;
            Ok((counters, rows, configured))
        })
        .await?;

    // The amount the owner chose to keep on the server (Settings), if any.
    let cap_override = state
        .db
        .call(|c| crate::payout::cap_override(c))
        .await
        .ok()
        .flatten();

    // Wallet figures straight from barkd (best effort).
    let wallet = match BarkdClient::from_app_config(&state.config) {
        Err(err) => json!({ "error": err.to_string() }),
        Ok(client) => {
            let mut out = serde_json::Map::new();
            let mut errors: Vec<String> = Vec::new();
            match client.balances().await {
                Ok(b) => {
                    out.insert("offchain_sats".into(), json!(b.offchain_sats));
                    out.insert("onchain_sats".into(), json!(b.onchain_sats));
                    out.insert("needs_refresh_sats".into(), json!(b.needs_refresh_sats));
                    out.insert("in_transit_sats".into(), json!(b.in_transit_sats()));
                    out.insert(
                        "hot_wallet_cap_sats".into(),
                        json!(cap_override.unwrap_or(state.config.hot_wallet_cap_sats)),
                    );
                }
                Err(e) => errors.push(format!("balances: {e}")),
            }
            if let Ok(c) = client.connected().await {
                if let Some(v) = c.connected {
                    out.insert("asp_connected".into(), json!(v));
                }
            }
            if !errors.is_empty() {
                out.insert("error".into(), json!(errors.join("; ")));
            }
            Value::Object(out)
        }
    };

    // Plain-language checklist for the Home tab (pure rules in health_checks).
    let usd_per_btc = state.usd_per_btc();
    let mut facts = crate::health_checks::Facts {
        usd_per_btc,
        price_age_secs: state.rates.newest_age_secs(),
        https: state.config.public_base_url.starts_with("https://"),
        local_only: {
            let u = state.config.public_base_url.to_lowercase();
            let host = u.split("://").nth(1).unwrap_or("");
            host.starts_with("localhost") || host.starts_with("127.") || host.starts_with("[::1]")
        },
        hot_wallet_cap_sats: state.config.hot_wallet_cap_sats,
        unmatched_open: counters["unmatched_payments_open"].as_i64().unwrap_or(0),
        webhooks_failed: counters["webhooks_failed"].as_i64().unwrap_or(0),
        webhook_configured,
        last_settlement_failed: recent_settlements
            .first()
            .map(|s| s["status"] == "failed")
            .unwrap_or(false),
        // Consecutive failed renewals, newest first (other kinds are skipped).
        renewals_failing: recent_settlements
            .iter()
            .filter(|s| s["kind"] == "renew")
            .take_while(|s| s["status"] == "failed")
            .count() as u32,
        ..Default::default()
    };
    facts.wallet_reachable = wallet.get("offchain_sats").is_some();
    facts.offchain_sats = wallet.get("offchain_sats").and_then(Value::as_i64);
    facts.needs_refresh_sats = wallet.get("needs_refresh_sats").and_then(Value::as_i64);
    facts.asp_connected = wallet.get("asp_connected").and_then(Value::as_bool);
    {
        let cfg = state.config.settlement_destination.clone();
        if let Ok(s) = state
            .db
            .call(move |c| crate::payout::status(c, cfg.as_deref()))
            .await
        {
            use crate::health_checks::Payout;
            if let Some(active) = s.active.as_deref() {
                facts.hot_wallet_cap_sats = s
                    .cap_override
                    .unwrap_or_else(|| state.config.sweep_cap_for(active));
            }
            let invalid = s
                .active
                .as_deref()
                .is_some_and(|a| crate::payout::check(a, &state.config.mode).is_err());
            facts.payout = match (s.active.is_some(), s.effective_at) {
                (_, Some(at)) if s.pending.is_some() => Payout::ChangePending(at),
                (true, _) if invalid => Payout::Invalid,
                (true, _) => Payout::Set,
                (false, _) => Payout::NotSet,
            };
        }
    }
    let checks = crate::health_checks::build(&facts);

    Ok(Json(json!({
        "checks": checks,
        "mode": state.config.mode,
        "onchain_rail_enabled": state.config.onchain_rail_enabled,
        "version": crate::VERSION,
        "counters": counters,
        "recent_settlements": recent_settlements,
        "wallet": wallet,
        // Dollars per bitcoin, for showing "(about $0.40)" next to sats; null when no price is known.
        "usd_per_btc": usd_per_btc,
        // Seconds since the price was last refreshed; null = never yet.
        "price_age_secs": state.rates.newest_age_secs(),
        // Which of the five price sources answered last time.
        "price_sources": state.rates.usd_sources(),
        "price_sources_total": crate::rates::default_sources().len(),
    })))
}
