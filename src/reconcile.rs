//! The weekly "does your own wallet agree?" check.
//!
//! A tampered or broken server could redirect payouts or show customers the
//! wrong address, and the server itself could never be trusted to say so. The
//! one check that does not depend on the server is the merchant's own wallet,
//! so the dashboard shows what the server CLAIMS for the last week and asks the
//! merchant to compare it with what their wallet actually received.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::settings;

pub const WINDOW_SECS: i64 = 7 * 24 * 3600;
const KEY: &str = "reconcile_checked_at";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub since: i64,
    /// Orders this server marked paid in the window (test-mode ones included).
    pub paid_orders: i64,
    pub paid_sats: i64,
    /// Sats this server says it sent on to the payout address in the window.
    pub swept_sats: i64,
    pub last_checked_at: Option<i64>,
    /// Time for the weekly check: there is something to compare and it has
    /// not been checked for a week.
    pub due: bool,
}

pub fn is_due(last_checked: Option<i64>, now: i64, paid_orders: i64, swept_sats: i64) -> bool {
    if paid_orders == 0 && swept_sats == 0 {
        return false;
    }
    match last_checked {
        None => true,
        Some(t) => now - t >= WINDOW_SECS,
    }
}

pub fn summary(conn: &Connection, now: i64) -> Result<Summary, AppError> {
    let since = now - WINDOW_SECS;
    // Every paid invoice counts, including ones made in test mode: a test-mode
    // server marks ALL its invoices as test, and its sweeps are counted too, so
    // leaving them out showed "0 orders paid" next to real sweeps.
    let (paid_orders, paid_sats): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(paid_sats), 0) FROM invoices
         WHERE status IN ('paid','overpaid') AND paid_at IS NOT NULL AND paid_at >= ?1",
        params![since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let swept_sats: i64 = conn.query_row(
        "SELECT COALESCE(SUM(amount_sats), 0) FROM settlements
         WHERE kind IN ('sweep_ark','sweep_onchain') AND status = 'broadcast' AND created_at >= ?1",
        params![since],
        |r| r.get(0),
    )?;
    let last_checked_at = settings::get(conn, KEY)?.and_then(|v| v.parse::<i64>().ok());
    Ok(Summary {
        since,
        paid_orders,
        paid_sats,
        swept_sats,
        last_checked_at,
        due: is_due(last_checked_at, now, paid_orders, swept_sats),
    })
}

pub fn mark_checked(conn: &Connection, now: i64) -> Result<(), AppError> {
    settings::set(conn, KEY, &now.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn due_only_when_there_is_something_to_compare_and_a_week_has_passed() {
        let now = 10_000_000;
        assert!(!is_due(None, now, 0, 0), "nothing to compare yet");
        assert!(is_due(None, now, 1, 0), "never checked");
        assert!(
            is_due(None, now, 0, 500),
            "money was sent on, never checked"
        );
        assert!(!is_due(Some(now - 3600), now, 5, 5), "checked an hour ago");
        assert!(is_due(Some(now - WINDOW_SECS), now, 5, 5), "a week ago");
    }

    #[tokio::test]
    async fn summary_counts_paid_orders_including_test_mode_and_sweeps_in_the_window_only() {
        let db = Db::open(":memory:").unwrap();
        db.call(|c| {
            let now = crate::util::now_unix();
            c.execute("INSERT INTO stores (id, name, created_at) VALUES ('s','S',1)", [])?;
            let add = |id: &str, status: &str, paid_at: Option<i64>, sats: i64, meta: &str| {
                c.execute(
                    "INSERT INTO invoices (id, store_id, status, amount_sats, ark_address, onchain_address,
                       expires_at, paid_at, paid_sats, metadata_json, created_at)
                     VALUES (?1,'s',?2,?4,?1,?1,?6,?3,?4,?5,1)",
                    params![id, status, paid_at, sats, meta, now + 100],
                )
            };
            add("a", "paid", Some(now - 60), 2000, "{}")?;
            add("b", "overpaid", Some(now - 120), 3000, "{}")?;
            add("old", "paid", Some(now - WINDOW_SECS - 10), 9999, "{}")?;
            add("test", "paid", Some(now - 30), 7777, r#"{"test":true}"#)?;
            add("open", "pending", None, 500, "{}")?;
            for (id, kind, st, sats, at) in [
                ("w1", "sweep_ark", "broadcast", 4000, now - 50),
                ("w2", "sweep_onchain", "failed", 8888, now - 40),
                ("w3", "renew", "broadcast", 1234, now - 40),
                ("w4", "sweep_ark", "broadcast", 5555, now - WINDOW_SECS - 5),
            ] {
                c.execute(
                    "INSERT INTO settlements (id, kind, amount_sats, destination, status, created_at)
                     VALUES (?1,?2,?4,'d',?3,?5)",
                    params![id, kind, st, sats, at],
                )?;
            }
            let s = summary(c, now)?;
            assert_eq!((s.paid_orders, s.paid_sats, s.swept_sats), (3, 12777, 4000));
            assert!(s.due && s.last_checked_at.is_none());
            mark_checked(c, now)?;
            let s = summary(c, now)?;
            assert!(!s.due && s.last_checked_at == Some(now));
            Ok(())
        })
        .await
        .unwrap();
    }
}
