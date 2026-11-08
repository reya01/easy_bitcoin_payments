//! Pause switch and shop-key address lock (see `crate::shopkey`; added because
//! of the PHP2Shell bugs that can take over a PHP shop). All need a
//! signed-in admin session plus the CSRF header.
//!
//! * `POST /admin/pause`        — `{paused: true|false}` stop or resume NEW payment requests
//! * `POST /admin/keylock/status` — the addresses the shop key is limited to, and the last refused one
//! * `POST /admin/keylock/save` — `{addresses: "..."}` set the limit (empty removes it)

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;
use crate::shopkey as sk;

fn audit(conn: &rusqlite::Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

#[derive(Deserialize)]
pub struct PauseRequest {
    pub paused: bool,
}

/// `POST /admin/pause`
pub async fn pause(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PauseRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    state
        .db
        .call(move |c| {
            sk::set_paused(c, req.paused)?;
            audit(
                c,
                if req.paused {
                    "admin.payments_paused"
                } else {
                    "admin.payments_resumed"
                },
                if req.paused {
                    "new payment requests paused"
                } else {
                    "new payment requests resumed"
                },
            );
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "paused": req.paused })))
}

fn view(c: &rusqlite::Connection) -> Result<Value, AppError> {
    let blocked = sk::last_blocked(c)?;
    Ok(json!({
        "addresses": sk::allowed(c)?,
        "last_blocked_address": blocked.as_ref().map(|b| b.0.clone()),
        "last_blocked_at": blocked.map(|b| b.1),
    }))
}

/// `POST /admin/keylock/status`
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    Ok(Json(state.db.call(|c| view(c)).await?))
}

#[derive(Deserialize)]
pub struct SaveRequest {
    pub addresses: String,
}

/// `POST /admin/keylock/save`
pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SaveRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let v = state
        .db
        .call(move |c| {
            let saved = sk::set_allowed(c, &req.addresses)?;
            audit(
                c,
                "admin.key_lock_changed",
                if saved.is_empty() {
                    "shop key limit removed"
                } else {
                    "shop key limited to listed addresses"
                },
            );
            view(c)
        })
        .await?;
    Ok(Json(v))
}
