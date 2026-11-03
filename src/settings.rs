//! The `settings` table: server-local key/value pairs (wizard progress,
//! the watcher's history cursor, later policies). Not store data, and never
//! secrets — the barkd token is env-only and the wallet mnemonic is never
//! stored anywhere.
//!
//! SCHEMA NOTE (do not "simplify" this back to two columns): the table is
//! created by migrations/0003_settlement.sql with an `updated_at INTEGER
//! NOT NULL` column (0004's `IF NOT EXISTS` copy then no-ops on every
//! migration-created database). The accessors below therefore write
//! updated_at, and the defensive `ensure_table` creates (and, on an old
//! two-column table, ALTERs to) the same shape so both paths converge.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;

/// Ensure the settings table exists (idempotent, cheap).
fn ensure_table(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (\
             key TEXT PRIMARY KEY, \
             value TEXT NOT NULL, \
             updated_at INTEGER NOT NULL DEFAULT 0)",
        [],
    )?;
    // Converge a table created by the OLD two-column defensive path: add
    // the column the migration schema (0003) already requires. Cheap check.
    let has_updated_at: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name = 'updated_at'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)?;
    if !has_updated_at {
        conn.execute(
            "ALTER TABLE settings ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
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
    // updated_at is NOT NULL in the migration schema (0003) — write it.
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, strftime('%s','now'))
         ON CONFLICT(key) DO UPDATE SET value = excluded.value,
            updated_at = excluded.updated_at",
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
