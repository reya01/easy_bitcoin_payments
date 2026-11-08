//! Late / unmatched payment ledger (beta hardening).
//!
//! The watcher records payments that arrive at the address of an invoice
//! that is no longer `pending` (expired, cancelled, already settled) in the
//! `unmatched_payments` table instead of dropping them (see
//! `invoice::record_unmatched`). The funds are in the till; the merchant
//! decides what to do (refund the customer, or fulfil the order manually).
//! These endpoints let a store see its own rows and mark them handled.
//!
//! Scoping: a row belongs to a store through `invoice_id -> invoices.store_id`.
//! Rows are never returned across stores.

use axum::extract::{Path, Query, State};
use axum::Json;
use rusqlite::params;
use serde::Serialize;

use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::routes::AppState;
use crate::util::{now_unix, to_rfc3339};

/// `?limit=` (default 100, max 500) and `?offset=` (R12).
#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UnmatchedPaymentView {
    pub id: String,
    pub invoice_id: Option<String>,
    pub order_id: Option<String>,
    pub rail: String,
    pub amount_sats: i64,
    /// `invoice_expired` | `invoice_closed` | `paid_after_expiry` | `onchain_rail_disabled`
    pub reason: String,
    pub detected_at: String,
    /// RFC 3339 once the merchant marked it handled; `null` while open.
    pub resolved_at: Option<String>,
}

/// `GET /api/v1/unmatched-payments` — this store's late/unmatched payments,
/// open ones first, newest first.
pub async fn list(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Query(page): Query<ListQuery>,
) -> Result<Json<Vec<UnmatchedPaymentView>>, AppError> {
    let store_id = auth.store.id.clone();
    let limit = page.limit.unwrap_or(100).clamp(1, 500);
    let offset = page.offset.unwrap_or(0).max(0);
    let rows = state
        .db
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT u.id, u.invoice_id, i.order_id, u.rail, u.amount_sats, u.reason,
                        u.detected_at, u.resolved_at
                 FROM unmatched_payments u
                 JOIN invoices i ON i.id = u.invoice_id
                 WHERE i.store_id = ?1
                 ORDER BY (u.resolved_at IS NOT NULL), u.detected_at DESC, u.id
                 LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt
                .query_map(params![store_id, limit, offset], |row| {
                    Ok(UnmatchedPaymentView {
                        id: row.get(0)?,
                        invoice_id: row.get(1)?,
                        order_id: row.get(2)?,
                        rail: row.get(3)?,
                        amount_sats: row.get(4)?,
                        reason: row.get(5)?,
                        detected_at: to_rfc3339(row.get::<_, i64>(6)?),
                        resolved_at: row.get::<_, Option<i64>>(7)?.map(to_rfc3339),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    Ok(Json(rows))
}

/// `POST /api/v1/unmatched-payments/{id}/resolve` — mark a row handled
/// (refunded / fulfilled manually). Idempotent; 404 for other stores' rows.
pub async fn resolve(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let store_id = auth.store.id.clone();
    let changed = state
        .db
        .call(move |conn| {
            let owned: i64 = conn.query_row(
                "SELECT COUNT(*) FROM unmatched_payments u
                 JOIN invoices i ON i.id = u.invoice_id
                 WHERE u.id = ?1 AND i.store_id = ?2",
                params![id, store_id],
                |row| row.get(0),
            )?;
            if owned == 0 {
                return Err(AppError::NotFound);
            }
            conn.execute(
                "UPDATE unmatched_payments SET resolved_at = ?1
                 WHERE id = ?2 AND resolved_at IS NULL",
                params![now_unix(), id],
            )?;
            crate::audit::record(conn, &store_id, "unmatched_payment.resolved", Some(&id))?;
            Ok(true)
        })
        .await?;
    Ok(Json(serde_json::json!({ "resolved": changed })))
}
