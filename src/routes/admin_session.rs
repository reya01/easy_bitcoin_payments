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
    state
        .config
        .public_base_url
        .trim()
        .to_ascii_lowercase()
        .starts_with("https://")
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
    let token = aa::token_from_headers(&headers);
    let (set, csrf) = state
        .db
        .call(move |c| {
            let csrf = match token {
                Some(t) => aa::lookup_session(c, &t)?,
                None => None,
            };
            Ok((aa::password_is_set(c)?, csrf))
        })
        .await?;
    Ok(Json(json!({
        "password_set": set,
        "authenticated": csrf.is_some(),
        "csrf": csrf,
    })))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
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
    let outcome = state
        .db
        .call(move |c| {
            if aa::check_password(c, &password)? {
                let s = aa::create_session(c)?;
                audit(c, "admin.login", "signed in");
                Ok(Some(s))
            } else {
                audit(c, "admin.login_failed", "wrong password");
                Ok(None)
            }
        })
        .await?;
    let Some(s) = outcome else {
        return Err(AppError::Unauthorized);
    };
    let body = Json(json!({ "authenticated": true, "csrf": s.csrf }));
    Ok(with_cookie(
        (StatusCode::OK, body).into_response(),
        &aa::set_cookie_value(&s.token, secure(&state)),
    ))
}

/// Require a live session AND the CSRF header; returns the session token.
async fn require_session(state: &AppState, headers: &HeaderMap) -> Result<String, AppError> {
    let token = aa::token_from_headers(headers).ok_or(AppError::Unauthorized)?;
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
    state
        .db
        .call(move |c| {
            if !aa::check_password(c, &current)? {
                return Err(AppError::Unauthorized);
            }
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
