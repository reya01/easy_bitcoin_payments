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
//! * `POST /admin/passkeys/stepup/begin|finish` — approve one protected action with a passkey
//!   (a signed-in session; the answer is a one-use ticket the action takes in place of a code).
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
const UNKNOWN_KEY: &str = "That passkey is not one of the passkeys for this server.";

/// Binding of the half-finished sign-in (password right, waiting for the passkey).
pub(crate) const LOGIN_BINDING: &str = "login";
/// Binding of an approval ticket (see `stepup_finish`): tied to the signed-in
/// session it was made in, so another session cannot spend it.
pub(crate) fn ticket_binding(session_token: &str) -> String {
    format!("ticket:{session_token}")
}
/// A protected action's code box takes `pk:` + a ticket instead of a 6-digit code.
pub(crate) const TICKET_PREFIX: &str = "pk:";

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
    let rp = rp_for_request(&state, &headers)?;
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
    check_second_step(&state, &headers, req.code).await?;
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
    let rp = rp_for_request(&state, &headers)?;
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
    check_second_step(&state, &headers, req.code).await?;
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
    check_second_step(&state, &headers, req.code).await?;
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

/// What the browser sends back after `navigator.credentials.get`.
#[derive(Deserialize)]
pub struct AssertionBody {
    /// The challenge from the options, handed back (base64url).
    pub challenge: String,
    /// The credential id (base64url).
    pub id: String,
    pub client_data_json: String,
    pub authenticator_data: String,
    pub signature: String,
}

/// The server address this request came in on (main or backup), for passkeys.
pub(crate) fn rp_for_request(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<passkey::Rp, AppError> {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok());
    passkey::rp_for_host(
        &state.config.public_base_url,
        &state.config.public_aliases,
        host,
    )
}

/// Check a passkey's answer to a challenge we issued (single use, tied to `binding`),
/// and remember the use. Every failure is a plain 400 (never a 401, which the page
/// reads as "signed out").
pub(crate) async fn check_answer(
    state: &AppState,
    purpose: Purpose,
    binding: &str,
    a: AssertionBody,
) -> Result<(), AppError> {
    let rp = passkey::rp_for_answer(
        &state.config.public_base_url,
        &state.config.public_aliases,
        &a.client_data_json,
    )?;
    let challenge = passkey::challenges()
        .take(&a.challenge, purpose, binding, now_unix())
        .ok_or_else(|| AppError::BadRequest(EXPIRED.into()))?;
    state
        .db
        .call(move |c| {
            let Some(stored) = passkey::get_by_credential(c, &a.id)? else {
                audit(c, "admin.passkey_failed", "unknown passkey");
                return Err(AppError::BadRequest(UNKNOWN_KEY.into()));
            };
            match passkey::verify_assertion(
                &rp,
                &challenge,
                false,
                &stored,
                &a.client_data_json,
                &a.authenticator_data,
                &a.signature,
            ) {
                Ok(ok) => {
                    passkey::record_use(c, stored.id, ok.new_sign_count, ok.backed_up, now_unix())?;
                    Ok(())
                }
                Err(e) => {
                    audit(c, "admin.passkey_failed", "passkey check failed");
                    Err(e)
                }
            }
        })
        .await
}

/// `POST /admin/passkeys/stepup/begin` — options for approving one protected action.
pub async fn stepup_begin(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    let token = require_session(&state, &headers).await?;
    let rp = rp_for_request(&state, &headers)?;
    let ids = state
        .db
        .call(|c| {
            Ok(passkey::list(c)?
                .into_iter()
                .map(|p| p.credential_id)
                .collect::<Vec<_>>())
        })
        .await?;
    if ids.is_empty() {
        return Err(AppError::BadRequest("No passkeys are set up.".into()));
    }
    let challenge = passkey::challenges().issue(Purpose::StepUp, &token, now_unix());
    Ok(Json(json!({
        "options": passkey::assertion_options(&rp, &challenge, &ids),
    })))
}

/// `POST /admin/passkeys/stepup/finish` — the passkey's answer in; a one-use ticket out
/// (two minutes). The action's own request hands it back as `pk:<ticket>` in its code
/// field, together with the admin password as always.
pub async fn stepup_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(a): Json<AssertionBody>,
) -> Result<Json<Value>, AppError> {
    let token = require_session(&state, &headers).await?;
    check_answer(&state, Purpose::StepUp, &token, a).await?;
    let ticket = passkey::challenges().issue(Purpose::StepUp, &ticket_binding(&token), now_unix());
    Ok(Json(json!({ "ticket": passkey::b64url_encode(&ticket) })))
}
