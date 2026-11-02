use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;

/// Embedded migrations, applied in order and tracked with `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_payments_txid_guard.sql"),
    include_str!("../migrations/0003_settlement.sql"),
    include_str!("../migrations/0004_settings.sql"),
];

/// A tiny SQLite handle.
///
/// Phase 0 uses a single connection behind a mutex, accessed only from
/// `spawn_blocking`. That is plenty for a single-merchant server and keeps the
/// dependency list minimal; Phase 1 can swap in a small pool without touching
/// call sites because everything goes through [`Db::call`].
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        if path != ":memory:" {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    let _ = std::fs::create_dir_all(parent);
                }
            }
        }

        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };

        Self::from_connection(conn)
    }

    pub fn from_connection(mut conn: Connection) -> Result<Self, rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a blocking database closure on the blocking thread pool.
    pub async fn call<T, F>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce(&mut Connection) -> Result<T, AppError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let mut guard = conn
                .lock()
                .map_err(|_| AppError::Internal("database lock poisoned".into()))?;
            f(&mut guard)
        })
        .await
        .map_err(|e| AppError::Internal(format!("database task failed: {e}")))?
    }
}

/// Read one row from the `settings` table (Phase 1: watcher cursor, policies).
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Upsert one row into the `settings` table.
pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![key, value, crate::util::now_unix()],
    )?;
    Ok(())
}

fn migrate(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = (index + 1) as i64;
        if current < version {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Db {
        Db::open(":memory:").expect("in-memory db")
    }

    #[tokio::test]
    async fn migrations_create_all_phase0_tables() {
        let db = memory_db();
        let tables: Vec<String> = db
            .call(|conn| {
                let mut stmt = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();

        for expected in ["api_keys", "invoices", "payments", "stores"] {
            assert!(
                tables.iter().any(|t| t == expected),
                "missing table {expected}, got {tables:?}"
            );
        }
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let db = memory_db();
        let version: i64 = db
            .call(|conn| Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);
    }

    #[tokio::test]
    async fn foreign_keys_are_enforced() {
        let db = memory_db();
        let result = db
            .call(|conn| {
                conn.execute(
                    "INSERT INTO invoices (id, store_id, status, amount_sats, ark_address, onchain_address, expires_at, created_at)
                     VALUES ('inv_x', 'missing-store', 'pending', 1000, 'ark1', 'bc1', 0, 0)",
                    [],
                )?;
                Ok(())
            })
            .await;
        assert!(result.is_err(), "orphan invoice must be rejected");
    }
}
