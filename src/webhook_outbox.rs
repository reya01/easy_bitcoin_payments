//! Durable webhook outbox (beta hardening).
//!
//! Flow: the watcher calls [`enqueue`] when an invoice changes state; the
//! single dispatcher task ([`run`]) wakes every few seconds, picks rows that
//! are `pending` and due, delivers them through
//! [`crate::webhook::WebhookEmitter`], and records the outcome.
//!
//! Guarantees / limits (read before changing):
//! * At-least-once. Receivers must dedupe on the `X-EBP-Idempotency-Key`
//!   header (unchanged contract).
//! * Survives restarts: state lives in `webhook_deliveries` (migration 0006).
//! * Retry schedule: first attempt immediately, then after
//!   [`BACKOFF_SECS`]; after [`MAX_ATTEMPTS`] total attempts the row is
//!   `failed` and stays for inspection. No UI / manual re-send yet.
//! * One dispatcher only (rows are not locked). Do not run two.
//! * The target URL is re-validated at delivery time
//!   ([`crate::webhook::validate_webhook_url`]); a rejected URL is a
//!   permanent failure, not a retry.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::db::Db;
use crate::error::AppError;
use crate::util::{new_id, now_unix};
use crate::webhook::{validate_webhook_url, WebhookEmitter};

/// Delay before attempt N+2 after attempt N+1 failed (seconds).
pub const BACKOFF_SECS: [i64; 5] = [30, 300, 1800, 7200, 43200];
/// Total attempts (first try + retries) before a row becomes `failed`.
pub const MAX_ATTEMPTS: i64 = 6;
/// Rows delivered per dispatcher pass.
const BATCH: i64 = 20;

/// Queue an event. Returns `true` if a new row was written, `false` if this
/// idempotency key was already queued (re-observed payment).
pub fn enqueue(
    conn: &Connection,
    store_id: &str,
    invoice_id: &str,
    event: &str,
    idempotency_key: &str,
    payload: &serde_json::Value,
) -> Result<bool, AppError> {
    let now = now_unix();
    let changed = conn.execute(
        "INSERT OR IGNORE INTO webhook_deliveries
            (id, store_id, invoice_id, event, idempotency_key, payload_json,
             status, attempts, next_attempt_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7)",
        params![
            new_id("whd"),
            store_id,
            invoice_id,
            event,
            idempotency_key,
            payload.to_string(),
            now
        ],
    )?;
    Ok(changed > 0)
}

struct Due {
    id: String,
    store_id: String,
    event: String,
    idempotency_key: String,
    payload_json: String,
    attempts: i64,
}

fn claim_due(conn: &Connection, now: i64) -> Result<Vec<Due>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, store_id, event, idempotency_key, payload_json, attempts
         FROM webhook_deliveries
         WHERE status = 'pending' AND next_attempt_at <= ?1
         ORDER BY next_attempt_at ASC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![now, BATCH], |row| {
            Ok(Due {
                id: row.get(0)?,
                store_id: row.get(1)?,
                event: row.get(2)?,
                idempotency_key: row.get(3)?,
                payload_json: row.get(4)?,
                attempts: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Record the outcome of one attempt. `Err((message, permanent))`.
fn record_attempt(
    conn: &Connection,
    id: &str,
    attempts_before: i64,
    outcome: &Result<(), (String, bool)>,
) -> Result<(), AppError> {
    let attempts = attempts_before + 1;
    let now = now_unix();
    match outcome {
        Ok(()) => {
            conn.execute(
                "UPDATE webhook_deliveries
                 SET status = 'delivered', attempts = ?1, last_error = NULL, delivered_at = ?2
                 WHERE id = ?3",
                params![attempts, now, id],
            )?;
        }
        Err((message, permanent)) => {
            if *permanent || attempts >= MAX_ATTEMPTS {
                conn.execute(
                    "UPDATE webhook_deliveries
                     SET status = 'failed', attempts = ?1, last_error = ?2
                     WHERE id = ?3",
                    params![attempts, message, id],
                )?;
            } else {
                let idx = ((attempts - 1) as usize).min(BACKOFF_SECS.len() - 1);
                conn.execute(
                    "UPDATE webhook_deliveries
                     SET attempts = ?1, last_error = ?2, next_attempt_at = ?3
                     WHERE id = ?4",
                    params![attempts, message, now + BACKOFF_SECS[idx], id],
                )?;
            }
        }
    }
    Ok(())
}

/// Deliver every due row once. Returns how many were delivered successfully.
pub async fn dispatch_due(
    db: &Db,
    emitter: &WebhookEmitter,
    allow_private_targets: bool,
) -> Result<usize, AppError> {
    let now = now_unix();
    let due = db.call(move |conn| claim_due(conn, now)).await?;
    let mut delivered = 0usize;

    for item in due {
        let store_id = item.store_id.clone();
        let store = db
            .call(move |conn| crate::store::get(conn, &store_id))
            .await?;
        let target = match store {
            Some(s) => match crate::store::webhook_secret_plain(&s) {
                Ok(secret) => match (s.webhook_url.clone(), secret) {
                    (Some(url), Some(secret)) => Some((url, secret)),
                    _ => None,
                },
                Err(err) => {
                    tracing::error!(error = %err, "webhook secret could not be decrypted; delivery will retry");
                    None
                }
            },
            None => None,
        };

        let outcome: Result<(), (String, bool)> = match target {
            None => Err(("store has no webhook configured".to_string(), true)),
            Some((url, secret)) => match validate_webhook_url(&url, allow_private_targets) {
                Err(reason) => Err((format!("webhook url rejected: {reason}"), true)),
                Ok(()) => {
                    let payload: serde_json::Value =
                        serde_json::from_str(&item.payload_json).unwrap_or(serde_json::Value::Null);
                    emitter
                        .emit(&url, &secret, &item.event, &item.idempotency_key, &payload)
                        .await
                        .map_err(|e| (e.to_string(), false))
                }
            },
        };

        if outcome.is_ok() {
            delivered += 1;
        } else if let Err((message, _)) = &outcome {
            tracing::warn!(event = %item.event, attempt = item.attempts + 1, error = %message, "webhook delivery failed");
        }

        let id = item.id.clone();
        let attempts = item.attempts;
        db.call(move |conn| record_attempt(conn, &id, attempts, &outcome))
            .await?;
    }
    Ok(delivered)
}

/// Run the dispatcher forever (one pass every 5 s). Each pass runs in its own
/// task so a panic is logged and the loop continues.
pub async fn run(db: Db, emitter: Arc<WebhookEmitter>, allow_private_targets: bool) {
    let mut ticker = tokio::time::interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let (db, emitter) = (db.clone(), Arc::clone(&emitter));
        match tokio::spawn(async move { dispatch_due(&db, &emitter, allow_private_targets).await })
            .await
        {
            Ok(Ok(n)) if n > 0 => tracing::info!(delivered = n, "webhooks delivered"),
            Ok(Ok(_)) => {}
            Ok(Err(err)) => tracing::warn!(error = %err, "webhook dispatch pass failed"),
            Err(join_err) => {
                tracing::error!(error = %join_err, "webhook dispatch PANICKED; continuing")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(conn: &Connection, where_sql: &str) -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM webhook_deliveries WHERE {where_sql}"),
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn enqueue_is_idempotent_per_key() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/0006_webhook_outbox.sql"))
            .unwrap();
        let payload = serde_json::json!({"event": "invoice.paid"});
        assert!(enqueue(
            &db,
            "s",
            "inv_1",
            "invoice.paid",
            "inv_1:invoice.paid:1000",
            &payload
        )
        .unwrap());
        assert!(!enqueue(
            &db,
            "s",
            "inv_1",
            "invoice.paid",
            "inv_1:invoice.paid:1000",
            &payload
        )
        .unwrap());
        assert_eq!(count(&db, "1=1"), 1);
    }

    #[test]
    fn failures_back_off_then_give_up() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/0006_webhook_outbox.sql"))
            .unwrap();
        enqueue(
            &db,
            "s",
            "inv_1",
            "invoice.paid",
            "k1",
            &serde_json::json!({}),
        )
        .unwrap();
        let id: String = db
            .query_row("SELECT id FROM webhook_deliveries", [], |r| r.get(0))
            .unwrap();

        let fail: Result<(), (String, bool)> = Err(("HTTP 500".into(), false));
        record_attempt(&db, &id, 0, &fail).unwrap();
        assert_eq!(count(&db, "status = 'pending' AND attempts = 1"), 1);
        let next: i64 = db
            .query_row("SELECT next_attempt_at FROM webhook_deliveries", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            next >= now_unix() + BACKOFF_SECS[0] - 1,
            "first retry waits ~30s"
        );

        // The final allowed attempt flips the row to failed.
        record_attempt(&db, &id, MAX_ATTEMPTS - 1, &fail).unwrap();
        assert_eq!(count(&db, "status = 'failed'"), 1);

        // A permanent failure fails immediately.
        enqueue(
            &db,
            "s",
            "inv_2",
            "invoice.paid",
            "k2",
            &serde_json::json!({}),
        )
        .unwrap();
        let id2: String = db
            .query_row(
                "SELECT id FROM webhook_deliveries WHERE idempotency_key='k2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        record_attempt(&db, &id2, 0, &Err(("bad url".into(), true))).unwrap();
        assert_eq!(
            count(&db, "idempotency_key = 'k2' AND status = 'failed'"),
            1
        );
    }

    #[test]
    fn success_marks_delivered() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/0006_webhook_outbox.sql"))
            .unwrap();
        enqueue(
            &db,
            "s",
            "inv_1",
            "invoice.paid",
            "k1",
            &serde_json::json!({}),
        )
        .unwrap();
        let id: String = db
            .query_row("SELECT id FROM webhook_deliveries", [], |r| r.get(0))
            .unwrap();
        record_attempt(&db, &id, 0, &Ok(())).unwrap();
        assert_eq!(
            count(&db, "status = 'delivered' AND delivered_at IS NOT NULL"),
            1
        );
    }
}
