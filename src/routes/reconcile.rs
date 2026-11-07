//! Weekly own-wallet check endpoints (see `crate::reconcile`). Signed-in admin
//! only; both are POST so the CSRF check applies.
//!
//! * `POST /admin/reconcile/status` — what the server claims for the last week
//! * `POST /admin/reconcile/done`   — "I checked my wallet, it matches"

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::reconcile as rc;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;

fn view(s: &rc::Summary) -> Value {
    json!({
        "days": rc::WINDOW_SECS / 86400,
        "since": s.since,
        "paid_orders": s.paid_orders,
        "paid_sats": s.paid_sats,
        "swept_sats": s.swept_sats,
        "last_checked_at": s.last_checked_at,
        "due": s.due,
    })
}

pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let s = state
        .db
        .call(|c| rc::summary(c, crate::util::now_unix()))
        .await?;
    Ok(Json(view(&s)))
}

pub async fn done(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let s = state
        .db
        .call(|c| {
            let now = crate::util::now_unix();
            rc::mark_checked(c, now)?;
            if let Ok(Some(store)) = crate::store::first(c) {
                let _ = crate::audit::record(
                    c,
                    &store.id,
                    "admin.wallet_check",
                    Some("weekly own-wallet check confirmed"),
                );
            }
            rc::summary(c, now)
        })
        .await?;
    Ok(Json(view(&s)))
}
