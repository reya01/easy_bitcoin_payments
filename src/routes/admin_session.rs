//! Admin login/logout/session endpoints (see `admin_auth` for the model).

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::admin_auth as aa;
use crate::error::AppError;
use crate::routes::AppState;

fn secure(state: &AppState) -> bool {
    aa::is_secure(&state.config.public_base_url)
}

fn with_cookie(mut resp: Response, cookie: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(cookie) {
        resp.headers_mut().append(header::SET_COOKIE, v);
    }
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

fn audit(conn: &rusqlite::Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

/// `GET /admin/session` — is there a password, and am I signed in?
pub async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AppError> {
    let token = aa::token_from_headers(&headers, secure(&state));
    let (set, csrf, twofa, declined, paused) = state
        .db
        .call(move |c| {
            let csrf = match token {
                Some(t) => aa::lookup_session(c, &t)?,
                None => None,
            };
            Ok((
                aa::password_is_set(c)?,
                csrf,
                crate::totp::is_enabled(c)?,
                crate::totp::is_declined(c)?,
                crate::shopkey::is_paused(c)?,
            ))
        })
        .await?;
    // The 2FA flags are only for signed-in pages (the sign-in screen asks for
    // a code after the password is right, so it never needs them).
    let signed_in = csrf.is_some();
    Ok(Json(json!({
        "password_set": set,
        "authenticated": signed_in,
        "csrf": csrf,
        "twofa": signed_in && twofa,
        "twofa_declined": signed_in && declined,
        "paused": signed_in && paused,
    })))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
    /// 6-digit code (or a recovery code); only needed when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

enum LoginOutcome {
    Session(aa::Session),
    NeedCode,
    WrongCode,
    Locked,
    Denied,
}

const LOCKED_MSG: &str = "Too many wrong codes. Wait 10 minutes and try again.";

/// Second-step check for actions that move money or hand out the backup.
/// Does nothing when two-step sign-in is off. Call it after the password check.
pub(crate) async fn check_second_step(
    state: &AppState,
    code: Option<String>,
) -> Result<(), AppError> {
    state
        .db
        .call(move |c| {
            if !crate::totp::is_enabled(c)? {
                return Ok(());
            }
            let Some(code) = code.filter(|x| !x.trim().is_empty()) else {
                return Err(AppError::BadRequest(
                    "Enter the 6-digit code from your authenticator app.".into(),
                ));
            };
            match crate::totp::verify(c, &code, crate::util::now_unix(), true)? {
                crate::totp::Second::Ok => Ok(()),
                crate::totp::Second::Wrong => {
                    audit(c, "admin.2fa_failed", "wrong code on a protected action");
                    Err(AppError::BadRequest(
                        "That code isn't right. Check your authenticator app and try again.".into(),
                    ))
                }
                crate::totp::Second::Locked => Err(AppError::BadRequest(LOCKED_MSG.into())),
            }
        })
        .await
}

/// `POST /admin/login`
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Response, AppError> {
    if req.password.len() > 1024 {
        return Err(AppError::Unauthorized);
    }
    let password = req.password;
    // 1. Read the stored hash (fast, inside the database lock).
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    // 2. The slow PBKDF2 check runs on a blocking thread OUTSIDE the lock, so
    //    a flood of login attempts cannot stall invoices and checkout.
    let ok = aa::verify_login_slow(stored, password).await?;
    // 3. Record the outcome (fast, inside the lock).
    let code = req.code;
    let outcome = state
        .db
        .call(move |c| {
            if !ok {
                audit(c, "admin.login_failed", "wrong password");
                return Ok(LoginOutcome::Denied);
            }
            // Password right. If two-step sign-in is on, a code is needed too.
            if crate::totp::is_enabled(c)? {
                let Some(code) = code.filter(|x| !x.trim().is_empty()) else {
                    return Ok(LoginOutcome::NeedCode);
                };
                match crate::totp::verify(c, &code, crate::util::now_unix(), false)? {
                    crate::totp::Second::Ok => {}
                    crate::totp::Second::Wrong => {
                        audit(c, "admin.login_failed", "wrong code");
                        return Ok(LoginOutcome::WrongCode);
                    }
                    crate::totp::Second::Locked => return Ok(LoginOutcome::Locked),
                }
            }
            let s = aa::create_session(c)?;
            audit(c, "admin.login", "signed in");
            Ok(LoginOutcome::Session(s))
        })
        .await?;
    let s = match outcome {
        LoginOutcome::Session(s) => s,
        LoginOutcome::NeedCode => {
            return Ok((StatusCode::OK, Json(json!({ "code_required": true }))).into_response())
        }
        LoginOutcome::WrongCode => {
            return Err(AppError::BadRequest(
                "That code isn't right. Check your authenticator app and try again, or use a recovery code.".into(),
            ))
        }
        LoginOutcome::Locked => return Err(AppError::BadRequest(LOCKED_MSG.into())),
        LoginOutcome::Denied => return Err(AppError::Unauthorized),
    };
    let body = Json(json!({ "authenticated": true, "csrf": s.csrf }));
    Ok(with_cookie(
        (StatusCode::OK, body).into_response(),
        &aa::set_cookie_value(&s.token, secure(&state)),
    ))
}

/// Require a live session AND the CSRF header; returns the session token.
pub(crate) async fn require_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    let token = aa::token_from_headers(headers, secure(state)).ok_or(AppError::Unauthorized)?;
    let t2 = token.clone();
    let csrf = state
        .db
        .call(move |c| aa::lookup_session(c, &t2))
        .await?
        .ok_or(AppError::Unauthorized)?;
    let supplied = headers
        .get(aa::CSRF_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::auth::ct_eq(&csrf, supplied) {
        return Err(AppError::Unauthorized);
    }
    Ok(token)
}

/// `POST /admin/logout`
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let token = require_session(&state, &headers).await?;
    state
        .db
        .call(move |c| {
            aa::delete_session(c, &token)?;
            audit(c, "admin.logout", "signed out");
            Ok(())
        })
        .await?;
    Ok(with_cookie(
        (StatusCode::OK, Json(json!({ "authenticated": false }))).into_response(),
        &aa::clear_cookie_value(secure(&state)),
    ))
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current: String,
    pub new: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/password` — change the password (signs every session out).
pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Response, AppError> {
    require_session(&state, &headers).await?;
    aa::validate_password(&req.new)?;
    let iters = state.config.password_hash_iterations;
    let (current, new) = (req.current, req.new);
    if current.len() > 1024 {
        return Err(AppError::BadRequest(
            "That is not your current password.".into(),
        ));
    }
    // The slow hash check runs outside the database lock, like sign-in does.
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let pw_ok = aa::verify_login_slow(stored, current).await?;
    if !pw_ok {
        // 400, not 401: a mistyped password must not look like "signed out".
        return Err(AppError::BadRequest(
            "That is not your current password.".into(),
        ));
    }
    check_second_step(&state, req.code).await?;
    state
        .db
        .call(move |c| {
            aa::set_password(c, &new, iters)?;
            audit(c, "admin.password_changed", "password changed");
            Ok(())
        })
        .await?;
    Ok(with_cookie(
        (StatusCode::OK, Json(json!({ "changed": true }))).into_response(),
        &aa::clear_cookie_value(secure(&state)),
    ))
}
