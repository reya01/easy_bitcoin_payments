//! Payout address endpoints (see `crate::payout`).
//!
//! * `GET  /api/v1/payout`         — current / pending address (signed in).
//! * `POST /admin/payout`          — change it (session + CSRF + admin password).
//! * `POST /admin/payout/cap`      — change how much the till keeps before the rest
//!   is sent on (session + CSRF + admin password, + second step when on).
//! * `POST /admin/payout/cancel`   — cancel a pending change (session + CSRF).
//! * `POST /admin/sweep`           — move the whole till to the payout address now
//!   (session + CSRF; it can only ever pay the saved payout address).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;

fn view(s: &crate::payout::Status, config: &crate::config::Config) -> Value {
    let delay_secs = config.payout_change_delay_secs;
    let kind = s.active.as_deref().map(|a| {
        if crate::config::is_ark_destination(a) {
            "ark"
        } else {
            "onchain"
        }
    });
    json!({
        "address": s.active,
        "kind": kind,
        "cap_sats": s.active.as_deref().map(|a| s.cap_override.unwrap_or_else(|| config.sweep_cap_for(a))),
        "cap_is_custom": s.cap_override.is_some(),
        "cap_min_sats": crate::payout::MIN_CAP_SATS,
        "cap_max_sats": crate::payout::MAX_CAP_SATS,
        "ark_cap_sats": s.cap_override.unwrap_or(config.hot_wallet_cap_sats),
        "onchain_cap_sats": s.cap_override.unwrap_or(config.onchain_sweep_cap_sats),
        "ark_default_cap_sats": config.hot_wallet_cap_sats,
        "onchain_default_cap_sats": config.onchain_sweep_cap_sats,
        "onchain_max_fee_sat_vb": config.onchain_sweep_max_fee_sat_vb,
        "pending_address": s.pending,
        "pending_effective_at": s.effective_at,
        "set_by_server_config": s.from_server_config,
        "change_delay_hours": delay_secs / 3600,
        "live": config.mode == "live",
    })
}

pub async fn get(State(state): State<AppState>, _auth: OwnerAuth) -> Result<Json<Value>, AppError> {
    let cfg = state.config.settlement_destination.clone();
    let s = state
        .db
        .call(move |c| crate::payout::status(c, cfg.as_deref()))
        .await?;
    Ok(Json(view(&s, &state.config)))
}

#[derive(Deserialize)]
pub struct ChangeRequest {
    pub address: String,
    pub admin_password: String,
    /// Authenticator code, needed only when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangeRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Validate first so a typo never costs a slow password check.
    let address = crate::payout::validate(&req.address, &state.config.mode)?;

    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let pw = req.admin_password;
    let ok = aa::verify_login_slow(stored, pw).await?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(&state, req.code).await?;

    let delay = state.config.payout_change_delay_secs;
    let cfg = state.config.settlement_destination.clone();
    let mode = state.config.mode.clone();
    let (change, s) = state
        .db
        .call(move |c| {
            let ch = crate::payout::request_change(c, &address, delay, cfg.as_deref(), &mode)?;
            let s = crate::payout::status(c, cfg.as_deref())?;
            Ok((ch, s))
        })
        .await?;
    let mut v = view(&s, &state.config);
    v["applied_now"] = json!(matches!(change, crate::payout::Change::Now));
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct CapRequest {
    /// The new amount to keep on the server, in sats. Absent or null = go back
    /// to the server's default.
    #[serde(default)]
    pub cap_sats: Option<i64>,
    pub admin_password: String,
    /// Authenticator code, needed only when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/payout/cap` — change how much the till keeps before the rest
/// is sent on. Same protection as the address (signed in, CSRF, admin
/// password, second step when on). It cannot redirect money, so it applies at
/// once and needs no waiting period.
pub async fn change_cap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CapRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Validate first so a typo never costs a slow password check.
    if let Some(v) = req.cap_sats {
        if !(crate::payout::MIN_CAP_SATS..=crate::payout::MAX_CAP_SATS).contains(&v) {
            return Err(AppError::BadRequest(format!(
                "the amount must be a whole number of sats between {} and {}",
                crate::payout::MIN_CAP_SATS,
                crate::payout::MAX_CAP_SATS
            )));
        }
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let ok = aa::verify_login_slow(stored, req.admin_password).await?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(&state, req.code).await?;

    let cfg = state.config.settlement_destination.clone();
    let cap = req.cap_sats;
    let s = state
        .db
        .call(move |c| {
            match cap {
                Some(v) => crate::payout::set_cap(c, v)?,
                None => crate::payout::clear_cap(c)?,
            }
            crate::payout::status(c, cfg.as_deref())
        })
        .await?;
    Ok(Json(view(&s, &state.config)))
}

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let cfg = state.config.settlement_destination.clone();
    let s = state
        .db
        .call(move |c| {
            crate::payout::cancel(c)?;
            crate::payout::status(c, cfg.as_deref())
        })
        .await?;
    Ok(Json(view(&s, &state.config)))
}

/// `POST /admin/sweep` — the merchant asks for the whole till to be sent to
/// the saved payout address right now (ignores the till cap and the on-chain
/// fee limit). It cannot redirect money: the destination is always the saved
/// payout address.
pub async fn sweep_now(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let cfg = state.config.settlement_destination.clone();
    let has_destination = state
        .db
        .call(move |c| crate::payout::effective(c, cfg.as_deref()))
        .await?
        .is_some_and(|d| !d.trim().is_empty());
    if !has_destination {
        return Err(AppError::BadRequest(
            "add a payout address first; there is nowhere to send the money yet".into(),
        ));
    }
    let client = crate::wallet::barkd::BarkdClient::from_app_config(&state.config)
        .map_err(AppError::from)?;
    let engine = crate::wallet::settlement::SettlementEngine::new(
        client,
        state.db.clone(),
        state.config.clone(),
    );
    let report = engine.sweep_now().await?;
    Ok(Json(json!({
        "swept_sats": report.swept_sats,
        "note": report.sweep_refused,
    })))
}
