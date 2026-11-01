//! The `settings` table: server-local key/value pairs (wizard progress,
//! later policies and notification config). Not store data, and never
//! secrets — the barkd token is env-only and the wallet mnemonic is never
//! stored anywhere (see docs/RECIPE_SETUP.md and docs/RECIPE_BACKUP.md).
//!
//! The table is created by `migrations/0002_settings.sql`; every accessor
//! also runs a defensive `CREATE TABLE IF NOT EXISTS` so a deployment
//! whose migration runner has not picked the file up yet converges on
//! first use instead of erroring.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;

/// Ensure the settings table exists (idempotent, cheap).
fn ensure_table(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        [],
    )?;
    Ok(())
}

/// Read one setting. Missing key → `None`.
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    ensure_table(conn)?;
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Write one setting (upsert).
pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    ensure_table(conn)?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Read one setting as a boolean ("true" / anything else).
pub fn get_bool(conn: &Connection, key: &str) -> Result<bool, AppError> {
    Ok(matches!(get(conn, key)?, Some(value) if value == "true"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_default_to_false() {
        let conn = Connection::open_in_memory().unwrap();
        assert!(!get_bool(&conn, "setup_complete").unwrap());
        assert_eq!(get(&conn, "missing").unwrap(), None);

        set(&conn, "setup_complete", "true").unwrap();
        assert!(get_bool(&conn, "setup_complete").unwrap());

        set(&conn, "setup_complete", "false").unwrap();
        assert!(!get_bool(&conn, "setup_complete").unwrap());
    }
}