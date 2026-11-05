//! Append-only audit trail of security-relevant actions (spec `audit_log`).
//! Never pass secret values into `detail`.

use rusqlite::{params, Connection};

use crate::error::AppError;
use crate::util::{new_id, now_unix};

pub fn record(
    conn: &Connection,
    store_id: &str,
    action: &str,
    detail: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO audit_log (id, store_id, action, detail, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![new_id("aud"), store_id, action, detail, now_unix()],
    )?;
    Ok(())
}
