use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::AppError;

/// Embedded migrations, applied in order and tracked with `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_payments_txid_guard.sql"),
    include_str!("../migrations/0003_settlement.sql"),
    include_str!("../migrations/0004_settings.sql"),
    include_str!("../migrations/0005_unmatched_payments.sql"),
    include_str!("../migrations/0006_webhook_outbox.sql"),
    include_str!("../migrations/0007_audit_log.sql"),
    include_str!("../migrations/0008_onchain_seen.sql"),
    include_str!("../migrations/0009_admin_login.sql"),
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
            let conn = Connection::open(path)?;
            backup_before_migrating(&conn, path)?;
            conn
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

// The settings table has ONE owner (src/settings.rs) — schema, ensure, and
// accessors. The watcher's cursor goes through these aliases; do not add a
// second settings implementation (that split is how the wizard's writes
// broke against the migration schema).
pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>, AppError> {
    crate::settings::get(conn, key)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), AppError> {
    crate::settings::set(conn, key, value)
}

fn plain_error(message: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(message.into())
}

/// How many pre-upgrade copies to keep.
const KEEP_BACKUPS: usize = 3;

/// Before an UPGRADE changes an existing database, keep a copy beside it in
/// `backups/` (the last three). A fresh database or one that is already up
/// to date is left alone. Fails closed: if the copy cannot be made, the
/// server does not touch the data.
fn backup_before_migrating(conn: &Connection, path: &str) -> Result<(), rusqlite::Error> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current <= 0 || current as usize >= MIGRATIONS.len() {
        return Ok(());
    }
    let dir = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| {
        plain_error(format!(
            "could not create the backups folder before upgrading: {e}"
        ))
    })?;
    let file = dir.join(format!(
        "ebp-{}-before-v{}.sqlite3",
        crate::util::now_unix(),
        MIGRATIONS.len()
    ));
    conn.execute("VACUUM INTO ?1", [file.to_string_lossy().as_ref()])
        .map_err(|e| {
            plain_error(format!(
                "could not back up the database before upgrading: {e}"
            ))
        })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
    prune_backups(&dir, KEEP_BACKUPS);
    tracing::info!(backup = %file.display(), "database backed up before upgrade");
    Ok(())
}

fn prune_backups(dir: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("ebp-") && n.ends_with(".sqlite3"))
        .collect();
    names.sort();
    names.reverse();
    for old in names.into_iter().skip(keep) {
        let _ = std::fs::remove_file(dir.join(old));
    }
}

fn migrate(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current as usize > MIGRATIONS.len() {
        return Err(plain_error(format!(
            "this database was written by a NEWER version of Easy Bitcoin Payments \
             (schema {current}, this version understands {}). Update the software; do not run an \
             older version on newer data.",
            MIGRATIONS.len()
        )));
    }

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
    #[test]
    fn upgrade_backup_and_downgrade_protection() {
        use super::*;
        let dir = std::env::temp_dir().join(format!("ebp-db-test-{}", crate::util::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ebp.sqlite3");
        let path_s = path.to_string_lossy().to_string();

        // A fresh database is never "backed up".
        let conn = Connection::open(&path).unwrap();
        backup_before_migrating(&conn, &path_s).unwrap();
        assert!(!dir.join("backups").exists());
        drop(conn);

        // An older database is copied before it is upgraded.
        let db = Db::open(&path_s).unwrap();
        drop(db);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", (MIGRATIONS.len() - 1) as i64)
            .unwrap();
        backup_before_migrating(&conn, &path_s).unwrap();
        let copies = std::fs::read_dir(dir.join("backups")).unwrap().count();
        assert_eq!(copies, 1);

        // Only the newest three are kept.
        for i in 0..5 {
            std::fs::write(
                dir.join("backups")
                    .join(format!("ebp-{i:010}-before-v1.sqlite3")),
                b"x",
            )
            .unwrap();
        }
        prune_backups(&dir.join("backups"), 3);
        assert_eq!(std::fs::read_dir(dir.join("backups")).unwrap().count(), 3);

        // A database from a newer version is refused with a plain message.
        let mut newer = Connection::open_in_memory().unwrap();
        newer
            .pragma_update(None, "user_version", (MIGRATIONS.len() + 1) as i64)
            .unwrap();
        let err = migrate(&mut newer).unwrap_err().to_string();
        assert!(err.contains("NEWER version"), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

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

        for expected in ["api_keys", "invoices", "payments", "settings", "stores"] {
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
