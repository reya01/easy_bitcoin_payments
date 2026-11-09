//! Two-step sign-in endpoints (see `crate::totp`). All need a signed-in session
//! plus the CSRF header.
//!
//! * `POST /admin/2fa/start`   — admin password in; a new secret + QR link out.
//! * `POST /admin/2fa/confirm` — a code from the app turns it on; 10 recovery codes out (once).
//! * `POST /admin/2fa/disable` — admin password + a code turns it off.
//! * `POST /admin/2fa/skip`    — "password only for now" (remembered, can be undone by turning it on).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::error::AppError;
use crate::routes::admin_session::{check_second_step, require_session};
use crate::routes::AppState;
use crate::totp;

fn audit(conn: &rusqlite::Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

/// Re-check the admin password (slow hash, off the database thread).
async fn password_ok(state: &AppState, password: String) -> Result<(), AppError> {
    if password.len() > 1024 {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let ok = aa::verify_login_slow(stored, password).await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ))
    }
}

#[derive(Deserialize)]
pub struct PasswordRequest {
    pub admin_password: String,
}

/// `POST /admin/2fa/start`
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PasswordRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    password_ok(&state, req.admin_password).await?;
    let (secret, account) = state
        .db
        .call(|c| {
            let secret = totp::begin_setup(c)?;
            let name = crate::store::first(c)?
                .map(|s| s.name)
                .unwrap_or_else(|| "Admin".into());
            Ok((secret, name))
        })
        .await?;
    Ok(Json(json!({
        "secret": secret,
        "uri": totp::otpauth_uri(&secret, &account),
    })))
}

#[derive(Deserialize)]
pub struct CodeRequest {
    pub code: String,
}

/// `POST /admin/2fa/confirm`
pub async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CodeRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let codes = state
        .db
        .call(move |c| {
            let codes = totp::confirm(c, &req.code, crate::util::now_unix())?;
            audit(c, "admin.2fa_enabled", "two-step sign-in turned on");
            Ok(codes)
        })
        .await?;
    Ok(Json(json!({ "enabled": true, "recovery_codes": codes })))
}

#[derive(Deserialize)]
pub struct DisableRequest {
    pub admin_password: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/2fa/disable`
pub async fn disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DisableRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    password_ok(&state, req.admin_password).await?;
    check_second_step(&state, req.code).await?;
    state
        .db
        .call(|c| {
            totp::disable(c)?;
            totp::set_declined(c, true)?;
            audit(c, "admin.2fa_disabled", "two-step sign-in turned off");
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "enabled": false })))
}

/// `POST /admin/2fa/skip`
pub async fn skip(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    state
        .db
        .call(|c| {
            if !totp::is_enabled(c)? {
                totp::set_declined(c, true)?;
                audit(c, "admin.2fa_skipped", "chose password only");
            }
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "skipped": true })))
}

/// `POST /admin/2fa/status`
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let (enabled, left) = state
        .db
        .call(|c| Ok((totp::is_enabled(c)?, totp::recovery_left(c)?)))
        .await?;
    Ok(Json(json!({ "enabled": enabled, "recovery_left": left })))
}
