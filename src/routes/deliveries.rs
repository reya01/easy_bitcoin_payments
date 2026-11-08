//! Webhook delivery log, one-tap re-send, and the audit trail (store-scoped).
//!
//! Closes the spec's Phase 2 "delivery log + re-send" for the API surface
//! (a dashboard UI is separate). Payload bodies are not returned.

use axum::extract::{Path, Query, State};
use axum::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::routes::AppState;
use crate::util::{now_unix, to_rfc3339};

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// Deliveries only: `pending` | `delivered` | `failed`.
    pub status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeliveryView {
    pub id: String,
    pub invoice_id: String,
    pub event: String,
    pub status: String,
    pub attempts: i64,
    pub next_attempt_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
}

/// `GET /api/v1/webhook-deliveries?status=&limit=&offset=`
pub async fn list(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Query(q): Query<PageQuery>,
) -> Result<Json<Vec<DeliveryView>>, AppError> {
    let store_id = auth.store.id.clone();
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let status = match q.status.as_deref() {
        None => None,
        Some(s @ ("pending" | "delivered" | "failed")) => Some(s.to_string()),
        Some(_) => {
            return Err(AppError::BadRequest(
                "status must be pending, delivered or failed".into(),
            ))
        }
    };
    let rows = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, invoice_id, event, status, attempts, next_attempt_at,
                        last_error, created_at, delivered_at
                 FROM webhook_deliveries
                 WHERE store_id = ?1 AND (?2 IS NULL OR status = ?2)
                 ORDER BY created_at DESC, id
                 LIMIT ?3 OFFSET ?4",
            )?;
            let rows = stmt
                .query_map(params![store_id, status, limit, offset], |r| {
                    let pending: String = r.get(3)?;
                    Ok(DeliveryView {
                        id: r.get(0)?,
                        invoice_id: r.get(1)?,
                        event: r.get(2)?,
                        attempts: r.get(4)?,
                        next_attempt_at: if pending == "pending" {
                            Some(to_rfc3339(r.get::<_, i64>(5)?))
                        } else {
                            None
                        },
                        status: pending,
                        last_error: r.get(6)?,
                        created_at: to_rfc3339(r.get::<_, i64>(7)?),
                        delivered_at: r.get::<_, Option<i64>>(8)?.map(to_rfc3339),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}

/// `POST /api/v1/webhook-deliveries/{id}/resend` — queue the delivery again
/// now (attempts reset). The idempotency key is unchanged, so a receiver that
/// already processed it should dedupe; this is for outages on their side.
pub async fn resend(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store_id = auth.store.id.clone();
    state
        .db
        .call(move |conn| {
            let changed = conn.execute(
                "UPDATE webhook_deliveries
                 SET status = 'pending', attempts = 0, next_attempt_at = ?1, last_error = NULL
                 WHERE id = ?2 AND store_id = ?3 AND status != 'pending'",
                params![now_unix(), id, store_id],
            )?;
            if changed == 0 {
                // Unknown, someone else's, or already pending: all 404/no-op.
                let exists: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM webhook_deliveries WHERE id = ?1 AND store_id = ?2",
                    params![id, store_id],
                    |r| r.get(0),
                )?;
                if exists == 0 {
                    return Err(AppError::NotFound);
                }
            }
            crate::audit::record(conn, &store_id, "webhook.resend", Some(&id))?;
            Ok(())
        })
        .await?;
    Ok(Json(serde_json::json!({ "queued": true })))
}

#[derive(Debug, Serialize)]
pub struct AuditView {
    pub id: String,
    pub action: String,
    pub detail: Option<String>,
    pub created_at: String,
}

/// `GET /api/v1/audit-log?limit=&offset=` — this store's security trail.
pub async fn audit(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Query(q): Query<PageQuery>,
) -> Result<Json<Vec<AuditView>>, AppError> {
    let store_id = auth.store.id.clone();
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let rows = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, action, detail, created_at FROM audit_log
                 WHERE store_id = ?1 ORDER BY created_at DESC, id LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt
                .query_map(params![store_id, limit, offset], |r| {
                    Ok(AuditView {
                        id: r.get(0)?,
                        action: r.get(1)?,
                        detail: r.get(2)?,
                        created_at: to_rfc3339(r.get::<_, i64>(3)?),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}
