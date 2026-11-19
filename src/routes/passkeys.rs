//! Passkey endpoints (see `crate::passkey`). All need a signed-in session plus the
//! CSRF header.
//!
//! * `POST /admin/passkeys/status`  — the list, and whether passkeys can be used here.
//! * `POST /admin/passkeys/begin`   — admin password + a second-step code in; options for
//!   the browser out. Needs the app code to be on first (while passkeys are new).
//! * `POST /admin/passkeys/finish`  — the browser's answer in; the passkey is saved.
//! * `POST /admin/passkeys/remove`  — admin password + a code turns one passkey off.
//! * `POST /admin/passkeys/disable` — admin password + a code turns all passkeys off.
//!
//! Everything that checks the password sits behind the password brake.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::error::AppError;
use crate::passkey::{self, Purpose, StoredPasskey};
use crate::routes::admin_session::{check_second_step, require_session};
use crate::routes::twofa::audit;
use crate::routes::AppState;
use crate::util::now_unix;

const NEEDS_APP_CODE: &str = "Turn on two-step sign-in with an app code first. Passkeys are added on top of it, and your backup codes cover both.";
const EXPIRED: &str = "That passkey request expired. Start again.";

/// Re-check the admin password (slow hash, off the database thread).
async fn password_ok(state: &AppState, password: String) -> Result<(), AppError> {
    if password.len() > 1024 {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    if aa::verify_login_slow(stored, password).await? {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ))
    }
}

fn view(p: &StoredPasskey) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        // Kept in a password manager or the phone's cloud (can be used on other devices).
        "synced": p.backed_up,
        "created_at": p.created_at,
        "last_used_at": p.last_used_at,
    })
}

fn views(keys: &[StoredPasskey]) -> Vec<Value> {
    keys.iter().map(view).collect()
}

/// `POST /admin/passkeys/status`
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let (two_step_on, keys) = state
        .db
        .call(|c| Ok((crate::totp::is_enabled(c)?, passkey::list(c)?)))
        .await?;
    let unavailable = passkey::rp_from_base_url(&state.config.public_base_url)
        .err()
        .map(|e| e.to_string());
    Ok(Json(json!({
        "two_step_on": two_step_on,
        "passkeys": views(&keys),
        "max": passkey::MAX_PASSKEYS,
        "unavailable": unavailable,
    })))
}

#[derive(Deserialize)]
pub struct BeginRequest {
    pub admin_password: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/passkeys/begin`
pub async fn begin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<BeginRequest>,
) -> Result<Json<Value>, AppError> {
    let token = require_session(&state, &headers).await?;
    let rp = passkey::rp_from_base_url(&state.config.public_base_url)?;
    let (on, have) = state
        .db
        .call(|c| Ok((crate::totp::is_enabled(c)?, passkey::count(c)?)))
        .await?;
    if !on {
        return Err(AppError::BadRequest(NEEDS_APP_CODE.into()));
    }
    if have >= passkey::MAX_PASSKEYS {
        return Err(AppError::Conflict(format!(
            "You already have {} passkeys. Remove one before adding another.",
            passkey::MAX_PASSKEYS
        )));
    }
    password_ok(&state, req.admin_password).await?;
    check_second_step(&state, req.code).await?;
    let (handle, account, existing) = state
        .db
        .call(|c| {
            let account = crate::store::first(c)?
                .map(|s| s.name)
                .unwrap_or_else(|| "Admin".into());
            let existing = passkey::list(c)?
                .into_iter()
                .map(|p| p.credential_id)
                .collect::<Vec<_>>();
            Ok((passkey::user_handle(c)?, account, existing))
        })
        .await?;
    let challenge = passkey::challenges().issue(Purpose::Register, &token, now_unix());
    Ok(Json(json!({
        "options": passkey::registration_options(&rp, &challenge, &handle, &account, &existing),
    })))
}

#[derive(Deserialize)]
pub struct FinishRequest {
    /// The challenge from `begin`, handed back (base64url).
    pub challenge: String,
    /// The credential id (base64url).
    pub id: String,
    pub client_data_json: String,
    pub attestation_object: String,
    #[serde(default)]
    pub name: String,
}

/// `POST /admin/passkeys/finish`
pub async fn finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<FinishRequest>,
) -> Result<Json<Value>, AppError> {
    let token = require_session(&state, &headers).await?;
    let rp = passkey::rp_from_base_url(&state.config.public_base_url)?;
    let challenge = passkey::challenges()
        .take(&req.challenge, Purpose::Register, &token, now_unix())
        .ok_or_else(|| AppError::BadRequest(EXPIRED.into()))?;
    let cred = passkey::verify_registration(
        &rp,
        &challenge,
        false,
        &req.client_data_json,
        &req.attestation_object,
        &req.id,
    )?;
    let name = req.name;
    let keys = state
        .db
        .call(move |c| {
            // The app code could have been turned off while the browser was asking.
            if !crate::totp::is_enabled(c)? {
                return Err(AppError::BadRequest(NEEDS_APP_CODE.into()));
            }
            passkey::add(c, &cred, &name, now_unix())?;
            audit(c, "admin.passkey_added", "passkey added");
            passkey::list(c)
        })
        .await?;
    Ok(Json(json!({ "passkeys": views(&keys) })))
}

#[derive(Deserialize)]
pub struct RemoveRequest {
    pub admin_password: String,
    #[serde(default)]
    pub code: Option<String>,
    pub id: i64,
}

/// `POST /admin/passkeys/remove`
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RemoveRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    password_ok(&state, req.admin_password).await?;
    check_second_step(&state, req.code).await?;
    let id = req.id;
    let keys = state
        .db
        .call(move |c| {
            if !passkey::remove(c, id)? {
                return Err(AppError::BadRequest("That passkey is already gone.".into()));
            }
            audit(c, "admin.passkey_removed", "passkey removed");
            passkey::list(c)
        })
        .await?;
    Ok(Json(json!({ "passkeys": views(&keys) })))
}

#[derive(Deserialize)]
pub struct DisableRequest {
    pub admin_password: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/passkeys/disable`
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
            passkey::remove_all(c)?;
            audit(c, "admin.passkeys_disabled", "passkeys turned off");
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "passkeys": Vec::<Value>::new() })))
}
