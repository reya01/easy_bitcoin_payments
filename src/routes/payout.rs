//! Payout address endpoints (see `crate::payout`).
//!
//! * `GET  /api/v1/payout`         — current / pending address (signed in).
//! * `POST /admin/payout`          — change it (session + CSRF + admin password).
//! * `POST /admin/payout/cancel`   — cancel a pending change (session + CSRF).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::auth::StoreAuth;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;

fn view(s: &crate::payout::Status, delay_secs: i64) -> Value {
    json!({
        "address": s.active,
        "pending_address": s.pending,
        "pending_effective_at": s.effective_at,
        "set_by_server_config": s.from_server_config,
        "change_delay_hours": delay_secs / 3600,
    })
}

pub async fn get(State(state): State<AppState>, _auth: StoreAuth) -> Result<Json<Value>, AppError> {
    let cfg = state.config.settlement_destination.clone();
    let s = state
        .db
        .call(move |c| crate::payout::status(c, cfg.as_deref()))
        .await?;
    Ok(Json(view(&s, state.config.payout_change_delay_secs)))
}

#[derive(Deserialize)]
pub struct ChangeRequest {
    pub address: String,
    pub admin_password: String,
}

pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangeRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Validate first so a typo never costs a slow password check.
    let address = crate::payout::validate(&req.address)?;

    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let pw = req.admin_password;
    let ok = tokio::task::spawn_blocking(move || aa::verify_login(stored.as_deref(), &pw))
        .await
        .map_err(|_| AppError::Internal("password check failed".into()))?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }

    let delay = state.config.payout_change_delay_secs;
    let cfg = state.config.settlement_destination.clone();
    let (change, s) = state
        .db
        .call(move |c| {
            let ch = crate::payout::request_change(c, &address, delay, cfg.as_deref())?;
            let s = crate::payout::status(c, cfg.as_deref())?;
            Ok((ch, s))
        })
        .await?;
    let mut v = view(&s, delay);
    v["applied_now"] = json!(matches!(change, crate::payout::Change::Now));
    Ok(Json(v))
}

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let cfg = state.config.settlement_destination.clone();
    let delay = state.config.payout_change_delay_secs;
    let s = state
        .db
        .call(move |c| {
            crate::payout::cancel(c)?;
            crate::payout::status(c, cfg.as_deref())
        })
        .await?;
    Ok(Json(view(&s, delay)))
}
