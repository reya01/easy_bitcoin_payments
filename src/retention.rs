//! Rolling cleanup so the database cannot grow without bound.
//!
//! Only operational logs are trimmed. NEVER touched: invoices, payments,
//! unmatched payments, settlements, stores, keys (the merchant's accounting
//! record). Deleted pages are reused by SQLite, so the file stops growing
//! even though it does not shrink.

use std::time::Duration;

use rusqlite::{params, Connection};

use crate::db::Db;
use crate::error::AppError;
use crate::util::now_unix;

const DAY: i64 = 86_400;
/// Failed-login rows are noise an attacker can generate: keep little.
const FAILED_LOGIN_DAYS: i64 = 14;
const FAILED_LOGIN_MAX_ROWS: i64 = 500;
const AUDIT_DAYS: i64 = 365;
const AUDIT_MAX_ROWS: i64 = 20_000;
const DELIVERED_WEBHOOK_DAYS: i64 = 90;
const FAILED_WEBHOOK_DAYS: i64 = 180;
const ONCHAIN_SEEN_DAYS: i64 = 90;
const SWEEP_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub audit: usize,
    pub webhooks: usize,
    pub onchain_seen: usize,
    pub sessions: usize,
}

pub fn sweep(conn: &Connection) -> Result<Swept, AppError> {
    sweep_at(conn, now_unix())
}

pub fn sweep_at(conn: &Connection, now: i64) -> Result<Swept, AppError> {
    let mut out = Swept::default();

    out.audit += conn.execute(
        "DELETE FROM audit_log WHERE action = 'admin.login_failed' AND created_at < ?1",
        params![now - FAILED_LOGIN_DAYS * DAY],
    )?;
    out.audit += conn.execute(
        "DELETE FROM audit_log WHERE action = 'admin.login_failed' AND id NOT IN (
           SELECT id FROM audit_log WHERE action = 'admin.login_failed'
           ORDER BY created_at DESC, rowid DESC LIMIT ?1)",
        params![FAILED_LOGIN_MAX_ROWS],
    )?;
    out.audit += conn.execute(
        "DELETE FROM audit_log WHERE created_at < ?1",
        params![now - AUDIT_DAYS * DAY],
    )?;
    out.audit += conn.execute(
        "DELETE FROM audit_log WHERE id NOT IN (
           SELECT id FROM audit_log ORDER BY created_at DESC, rowid DESC LIMIT ?1)",
        params![AUDIT_MAX_ROWS],
    )?;

    out.webhooks += conn.execute(
        "DELETE FROM webhook_deliveries WHERE status = 'delivered' AND created_at < ?1",
        params![now - DELIVERED_WEBHOOK_DAYS * DAY],
    )?;
    out.webhooks += conn.execute(
        "DELETE FROM webhook_deliveries WHERE status = 'failed' AND created_at < ?1",
        params![now - FAILED_WEBHOOK_DAYS * DAY],
    )?;

    // Only for invoices that are finished; an open invoice still needs them.
    out.onchain_seen += conn.execute(
        "DELETE FROM onchain_seen WHERE first_seen_at < ?1 AND invoice_id IN (
           SELECT id FROM invoices WHERE status NOT IN ('pending', 'underpaid'))",
        params![now - ONCHAIN_SEEN_DAYS * DAY],
    )?;

    out.sessions += conn.execute(
        "DELETE FROM admin_sessions WHERE expires_at <= ?1 OR last_seen <= ?2",
        params![now, now - 12 * 3600],
    )?;
    Ok(out)
}

/// Sweep at startup and every six hours.
pub async fn run(db: Db) {
    loop {
        match db.call(|c| sweep(c)).await {
            Ok(s) if s != Swept::default() => {
                tracing::info!(?s, "retention sweep removed old log rows")
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err, "retention sweep failed"),
        }
        tokio::time::sleep(SWEEP_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
        for m in [
            include_str!("../migrations/0006_webhook_outbox.sql"),
            include_str!("../migrations/0007_audit_log.sql"),
            include_str!("../migrations/0009_admin_login.sql"),
        ] {
            db.execute_batch(m).unwrap();
        }
        db.execute_batch(
            "CREATE TABLE invoices (id TEXT PRIMARY KEY, status TEXT NOT NULL);
             CREATE TABLE onchain_seen (invoice_id TEXT, txid TEXT, vout INTEGER, amount_sats INTEGER,
               first_seen_at INTEGER, confirmations INTEGER);",
        )
        .unwrap();
        db
    }

    fn audit(c: &Connection, id: &str, action: &str, at: i64) {
        c.execute(
            "INSERT INTO audit_log (id, store_id, action, created_at) VALUES (?1, 's', ?2, ?3)",
            params![id, action, at],
        )
        .unwrap();
    }

    fn count(c: &Connection, sql: &str) -> i64 {
        c.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn trims_old_logs_and_keeps_recent_and_open_work() {
        let c = conn();
        let now = 10_000_000_i64;
        audit(&c, "old_fail", "admin.login_failed", now - 30 * DAY);
        audit(&c, "new_fail", "admin.login_failed", now - DAY);
        audit(&c, "ancient", "store.api_keys_rotated", now - 400 * DAY);
        audit(&c, "recent", "store.api_keys_rotated", now - 10 * DAY);
        for (id, status, age) in [
            ("d_old", "delivered", 100),
            ("d_new", "delivered", 10),
            ("f_old", "failed", 200),
            ("f_new", "failed", 100),
            ("p_old", "pending", 300),
        ] {
            c.execute(
                "INSERT INTO webhook_deliveries (id, store_id, invoice_id, event, idempotency_key, payload_json, status, next_attempt_at, created_at)
                 VALUES (?1,'s','i','e',?1,'{}',?2,0,?3)",
                params![id, status, now - age * DAY],
            )
            .unwrap();
        }
        c.execute_batch("INSERT INTO invoices VALUES ('paid','paid'),('open','pending');")
            .unwrap();
        for inv in ["paid", "open"] {
            c.execute(
                "INSERT INTO onchain_seen VALUES (?1,'t',0,1,?2,1)",
                params![inv, now - 100 * DAY],
            )
            .unwrap();
        }

        let s = sweep_at(&c, now).unwrap();
        assert_eq!(s.audit, 2);
        assert_eq!(s.webhooks, 2);
        assert_eq!(s.onchain_seen, 1);
        let left: Vec<String> = c
            .prepare("SELECT id FROM audit_log ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(left, vec!["new_fail", "recent"]);
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'pending'"
            ),
            1,
            "pending deliveries are never swept"
        );
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM onchain_seen WHERE invoice_id = 'open'"
            ),
            1
        );
        // Idempotent.
        assert_eq!(sweep_at(&c, now).unwrap(), Swept::default());
    }

    #[test]
    fn caps_failed_login_flood_and_total_audit_rows() {
        let c = conn();
        let now = 10_000_000_i64;
        let tx = c.unchecked_transaction().unwrap();
        for i in 0..(FAILED_LOGIN_MAX_ROWS + 50) {
            audit(
                &c,
                &format!("f{i:05}"),
                "admin.login_failed",
                now - 1000 + i,
            );
        }
        tx.commit().unwrap();
        sweep_at(&c, now).unwrap();
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM audit_log WHERE action = 'admin.login_failed'"
            ),
            FAILED_LOGIN_MAX_ROWS
        );
        // The newest survive.
        assert_eq!(
            count(&c, "SELECT COUNT(*) FROM audit_log WHERE id = 'f00549'"),
            1
        );
    }
}
