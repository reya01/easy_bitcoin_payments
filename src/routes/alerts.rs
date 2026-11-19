//! Phone alert endpoints (see `crate::alerts`). All need a signed-in admin
//! session; the topic name is only ever returned here.
//!
//! * `POST /admin/alerts/status`   — state, topic and server (POST so the CSRF check applies)
//! * `POST /admin/alerts/enable`   — turn on (optional own `server`; a code is
//!   needed only for an own server, and only when two-step sign-in is on)
//! * `POST /admin/alerts/disable`  — opt out (a code is needed only to turn off
//!   alerts that are working, and only when two-step sign-in is on)
//! * `POST /admin/alerts/activity` — how to hear about payments: daily (default), each or off
//! * `POST /admin/alerts/test`     — send a test alert

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::alerts as al;
use crate::error::AppError;
use crate::routes::admin_session::{check_second_step, require_session};
use crate::routes::AppState;

async fn view(state: &AppState) -> Result<Json<Value>, AppError> {
    let v = state
        .db
        .call(|c| {
            let st = al::state(c)?;
            let topic = al::topic(c)?;
            let server = al::server(c)?;
            Ok(json!({
                "state": st.as_str(),
                "topic": if st == al::State::On { topic } else { None },
                "server": server,
                "default_server": al::DEFAULT_SERVER,
                "activity": al::activity(c)?.as_str(),
            }))
        })
        .await?;
    Ok(Json(v))
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    view(&state).await
}

#[derive(Deserialize, Default)]
pub struct EnableRequest {
    pub server: Option<String>,
    /// Authenticator code, needed only to use an own server.
    #[serde(default)]
    pub code: Option<String>,
}

pub async fn enable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EnableRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Sending alerts to another server is the one change here that could
    // hide trouble from the owner, so it asks for the second step. Turning
    // alerts on with the default server never does.
    let own_server = req.server.as_deref().is_some_and(|s| !s.trim().is_empty());
    if own_server {
        check_second_step(&state, &headers, req.code.clone()).await?;
    }
    let server = req.server;
    state
        .db
        .call(move |c| al::enable(c, server.as_deref()))
        .await?;
    view(&state).await
}

#[derive(Deserialize)]
pub struct ActivityRequest {
    pub mode: String,
}

/// `POST /admin/alerts/activity` — `daily`, `each` or `off`.
pub async fn activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ActivityRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    state
        .db
        .call(move |c| al::set_activity(c, &req.mode))
        .await?;
    view(&state).await
}

#[derive(Deserialize, Default)]
pub struct DisableRequest {
    /// Authenticator code, needed only to turn off alerts that are working.
    #[serde(default)]
    pub code: Option<String>,
}

pub async fn disable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DisableRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Saying "no thanks" to the first question, or turning off alerts that were
    // already off, needs nothing. Switching off alerts that work asks for the
    // second step (when it is on), because silence is what an attacker wants.
    let working = state
        .db
        .call(|c| Ok(al::state(c)? == al::State::On))
        .await?;
    if working {
        check_second_step(&state, &headers, req.code).await?;
    }
    state.db.call(|c| al::disable(c)).await?;
    view(&state).await
}

pub async fn test(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    al::send_test(&state.db).await?;
    Ok(Json(json!({ "sent": true })))
}
