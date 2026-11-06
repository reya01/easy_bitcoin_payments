//! Phone alert endpoints (see `crate::alerts`). All need a signed-in admin
//! session; the topic name is only ever returned here.
//!
//! * `POST /admin/alerts/status`   — state, topic and server (POST so the CSRF check applies)
//! * `POST /admin/alerts/enable`   — turn on (optional own `server`)
//! * `POST /admin/alerts/disable`  — opt out
//! * `POST /admin/alerts/test`     — send a test alert

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::alerts as al;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
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
}

pub async fn enable(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<EnableRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let server = req.server;
    state
        .db
        .call(move |c| al::enable(c, server.as_deref()))
        .await?;
    view(&state).await
}

pub async fn disable(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
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
