//! Encrypted backup bundle (admin "Download a backup" and `ebp backup ...`).
//!
//! What a bundle holds:
//! * `ebp.sqlite3` — a consistent snapshot of the EBP database (invoices,
//!   stores, payment history, settings). Signed-in admin sessions are
//!   removed from the copy. Secrets inside it (webhook secrets) stay
//!   encrypted with the server's key-encryption key, which is NOT in the
//!   bundle: keep that key separately.
//! * `ebp.key` — the key that protects those stored secrets, so ONE file
//!   restores everything (the bundle is itself locked with the admin
//!   password, and it already holds the wallet folder, which matters more).
//! * `barkd/...` — the wallet's data folder, ONLY in managed-barkd mode
//!   (when EBP runs barkd itself). With a separate barkd container EBP
//!   cannot see those files; the bundle says so.
//!
//! The recovery words remain the real backup of the money. This bundle is
//! for speed and history.
//!
//! Format (everything after the header is one ChaCha20-Poly1305 message):
//! `EBPBAK1\n` | iterations u32 BE | salt 16 | nonce 12 | ciphertext+tag.
//! The 40-byte header is authenticated as associated data. The key is
//! PBKDF2-HMAC-SHA256(backup password, salt, iterations). The backup
//! password is chosen at download time and is never stored.
//! Plaintext: count u32 BE, then per entry: name_len u16 BE, name,
//! data_len u64 BE, data.

use std::path::{Path, PathBuf};

use rand::RngCore;

use crate::error::AppError;

const MAGIC: &[u8; 8] = b"EBPBAK1\n";
const HEADER_LEN: usize = 8 + 4 + 16 + 12;
pub const DEFAULT_ITERATIONS: u32 = 600_000;
const MIN_ITERATIONS: u32 = 1_000;
const MAX_ITERATIONS: u32 = 5_000_000;
pub const MIN_PASSWORD_LEN: usize = 16;
pub const MAX_PASSWORD_LEN: usize = 200;
/// Refuse to build or open anything bigger than this (the bundle is held in
/// memory; a wallet database is megabytes at most).
pub const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

pub fn validate_password(p: &str) -> Result<(), AppError> {
    let n = p.chars().count();
    if n < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "choose a backup password of at least {MIN_PASSWORD_LEN} characters (a few random words works well)"
        )));
    }
    if n > MAX_PASSWORD_LEN {
        return Err(AppError::BadRequest(
            "that backup password is too long".into(),
        ));
    }
    Ok(())
}

/// Entry names are relative, plain and short: no `..`, no absolute paths.
pub fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 200 || name.starts_with('/') {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
    {
        return false;
    }
    name.split('/')
        .all(|c| !c.is_empty() && c != "." && c != "..")
}

fn encode(entries: &[Entry]) -> Result<Vec<u8>, AppError> {
    let total: usize = entries.iter().map(|e| e.data.len()).sum();
    if total > MAX_TOTAL_BYTES || entries.len() > MAX_ENTRIES {
        return Err(AppError::BadRequest(
            "the data is too large to back up this way; snapshot the whole server volume instead"
                .into(),
        ));
    }
    let mut out = Vec::with_capacity(total + 64 * entries.len() + 4);
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for e in entries {
        out.extend_from_slice(&(e.name.len() as u16).to_be_bytes());
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&(e.data.len() as u64).to_be_bytes());
        out.extend_from_slice(&e.data);
    }
    Ok(out)
}

fn take<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8], AppError> {
    if buf.len() < n {
        return Err(corrupt());
    }
    let (a, b) = buf.split_at(n);
    *buf = b;
    Ok(a)
}

fn corrupt() -> AppError {
    AppError::BadRequest("this backup file is damaged or incomplete".into())
}

fn decode(mut buf: &[u8]) -> Result<Vec<Entry>, AppError> {
    let count = u32::from_be_bytes(take(&mut buf, 4)?.try_into().unwrap()) as usize;
    if count > MAX_ENTRIES {
        return Err(corrupt());
    }
    let mut out = Vec::new();
    for _ in 0..count {
        let nl = u16::from_be_bytes(take(&mut buf, 2)?.try_into().unwrap()) as usize;
        let name = std::str::from_utf8(take(&mut buf, nl)?)
            .map_err(|_| corrupt())?
            .to_string();
        if !valid_name(&name) {
            return Err(AppError::BadRequest(
                "this backup contains an unsafe file name and was not opened".into(),
            ));
        }
        let dl = u64::from_be_bytes(take(&mut buf, 8)?.try_into().unwrap());
        let dl = usize::try_from(dl).map_err(|_| corrupt())?;
        let data = take(&mut buf, dl)?.to_vec();
        out.push(Entry { name, data });
    }
    if !buf.is_empty() {
        return Err(corrupt());
    }
    Ok(out)
}

fn derive_key(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    crate::admin_auth::pbkdf2(password.as_bytes(), salt, iterations)
}

/// Encrypt `entries` under `password` (slow: PBKDF2; run on a blocking thread).
pub fn seal(entries: &[Entry], password: &str) -> Result<Vec<u8>, AppError> {
    seal_with(entries, password, DEFAULT_ITERATIONS)
}

pub fn seal_with(entries: &[Entry], password: &str, iterations: u32) -> Result<Vec<u8>, AppError> {
    let plain = encode(entries)?;
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let mut out = Vec::with_capacity(HEADER_LEN + plain.len() + 16);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&iterations.to_be_bytes());
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    let ct = crate::aead::seal(
        &derive_key(password, &salt, iterations),
        &nonce,
        &out[..HEADER_LEN],
        &plain,
    );
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt and parse a bundle. A wrong password and a damaged file look the
/// same on purpose (the cipher cannot tell them apart).
pub fn open(bytes: &[u8], password: &str) -> Result<Vec<Entry>, AppError> {
    decode(&decrypt(bytes, password)?)
}

/// Check the header and decrypt; the plaintext is the encoded entry list.
fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, AppError> {
    if bytes.len() < HEADER_LEN + 16 || &bytes[..8] != MAGIC {
        return Err(AppError::BadRequest(
            "this is not an Easy Bitcoin Payments backup file".into(),
        ));
    }
    let iterations = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
        return Err(corrupt());
    }
    let salt = &bytes[12..28];
    let nonce = &bytes[28..40];
    let nonce: [u8; 12] = nonce.try_into().map_err(|_| corrupt())?;
    let plain = crate::aead::open(
        &derive_key(password, salt, iterations),
        &nonce,
        &bytes[..HEADER_LEN],
        &bytes[HEADER_LEN..],
    )
    .ok_or_else(|| {
        AppError::BadRequest(
            "could not open the backup: wrong backup password, or the file is damaged".into(),
        )
    })?;
    Ok(plain)
}

/// Open a bundle and list what it holds (names and sizes) without copying
/// the files out: for the admin page's "Check a backup".
pub fn inspect(bytes: &[u8], password: &str) -> Result<Vec<(String, usize)>, AppError> {
    let plain = decrypt(bytes, password)?;
    let mut buf: &[u8] = &plain;
    let count = u32::from_be_bytes(take(&mut buf, 4)?.try_into().unwrap()) as usize;
    if count > MAX_ENTRIES {
        return Err(corrupt());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let nl = u16::from_be_bytes(take(&mut buf, 2)?.try_into().unwrap()) as usize;
        let name = std::str::from_utf8(take(&mut buf, nl)?)
            .map_err(|_| corrupt())?
            .to_string();
        if !valid_name(&name) {
            return Err(AppError::BadRequest(
                "this backup contains an unsafe file name and was not opened".into(),
            ));
        }
        let dl = u64::from_be_bytes(take(&mut buf, 8)?.try_into().unwrap());
        let dl = usize::try_from(dl).map_err(|_| corrupt())?;
        take(&mut buf, dl)?;
        out.push((name, dl));
    }
    if !buf.is_empty() {
        return Err(corrupt());
    }
    Ok(out)
}

/// Files a restore does not need: lock files (they belong to a running
/// process) and the daemon's debug log.
fn is_not_worth_backing_up(name: &str) -> bool {
    matches!(name, "LOCK" | "barkd.lock" | "debug.log")
}

fn is_sqlite_name(n: &str) -> bool {
    let n = n.to_ascii_lowercase();
    n.ends_with(".sqlite") || n.ends_with(".sqlite3") || n.ends_with(".db")
}

fn is_sqlite_sidecar(n: &str) -> bool {
    let n = n.to_ascii_lowercase();
    n.ends_with("-wal") || n.ends_with("-shm") || n.ends_with("-journal")
}

/// Consistent copy of a SQLite file (`VACUUM INTO` reads a snapshot, so a
/// running writer cannot hand us a half-written file).
fn sqlite_snapshot(src: &Path, tmp_dir: &Path) -> Result<Vec<u8>, AppError> {
    let tmp = tmp_dir.join(format!(".ebp-backup-{}.tmp", crate::util::random_hex(8)));
    let result = (|| -> Result<Vec<u8>, AppError> {
        let conn = rusqlite::Connection::open_with_flags(
            src,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])?;
        std::fs::read(&tmp).map_err(|e| AppError::Internal(format!("backup read failed: {e}")))
    })();
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Snapshot of the EBP database as a bundle entry, with admin sessions
/// removed. `conn` is the live connection (call inside `Db::call`).
pub fn database_entry(conn: &rusqlite::Connection) -> Result<Entry, AppError> {
    // Temporary copy beside the live database (a private folder); for an
    // in-memory database, the system temp folder.
    let dir: PathBuf = conn
        .path()
        .map(Path::new)
        .filter(|p| !p.as_os_str().is_empty() && p.as_os_str() != ":memory:")
        .and_then(|p| p.parent())
        .map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p.to_path_buf()
            }
        })
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let tmp = dir.join(format!(".ebp-backup-{}.tmp", crate::util::random_hex(8)));
    let result = (|| -> Result<Vec<u8>, AppError> {
        conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])?;
        {
            let copy = rusqlite::Connection::open(&tmp)?;
            // A restored copy must not carry live logins.
            copy.execute("DELETE FROM admin_sessions", [])?;
            // Two-step sign-in stays out of backups: the file also holds the
            // key that unlocks its secret, and the password alone opens the
            // file, so keeping it would let the password undo the second step.
            // A restored server starts with two-step sign-in off and offers
            // to turn it on again.
            copy.execute("DELETE FROM admin_totp", [])?;
            copy.execute("DELETE FROM admin_recovery_codes", [])?;
        }
        std::fs::read(&tmp).map_err(|e| AppError::Internal(format!("backup read failed: {e}")))
    })();
    let _ = std::fs::remove_file(&tmp);
    Ok(Entry {
        name: "ebp.sqlite3".into(),
        data: result?,
    })
}

/// The at-rest encryption key as a bundle entry (hex, same format as the
/// `ebp.key` file the server reads when no environment key is set).
pub fn key_entry() -> Entry {
    Entry {
        name: "ebp.key".into(),
        data: crate::secrets::get().key_hex().into_bytes(),
    }
}

/// The wallet folder as bundle entries (`barkd/<relative path>`). SQLite
/// files are copied as consistent snapshots; other regular files as-is;
/// symlinks and special files are skipped.
pub fn wallet_entries(datadir: &Path) -> Result<Vec<Entry>, AppError> {
    let mut out = Vec::new();
    let mut total = 0usize;
    // Temporary snapshots go beside (not inside) the wallet folder: it is the
    // EBP data folder, which is private, and barkd never sees the temp file.
    let tmp_dir = datadir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    walk(datadir, datadir, tmp_dir, &mut out, &mut total, 0)?;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    tmp_dir: &Path,
    out: &mut Vec<Entry>,
    total: &mut usize,
    depth: usize,
) -> Result<(), AppError> {
    if depth > 8 {
        return Ok(());
    }
    let read = std::fs::read_dir(dir)
        .map_err(|e| AppError::Internal(format!("could not read the wallet folder: {e}")))?;
    for item in read {
        let item = item.map_err(|e| AppError::Internal(format!("wallet folder: {e}")))?;
        let ft = item
            .file_type()
            .map_err(|e| AppError::Internal(format!("wallet folder: {e}")))?;
        let path = item.path();
        let fname = item.file_name().to_string_lossy().to_string();
        if ft.is_dir() {
            walk(root, &path, tmp_dir, out, total, depth + 1)?;
            continue;
        }
        if !ft.is_file() || is_sqlite_sidecar(&fname) || is_not_worth_backing_up(&fname) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| AppError::Internal("wallet folder path error".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let name = format!("barkd/{rel}");
        if !valid_name(&name) {
            return Err(AppError::BadRequest(format!(
                "the wallet folder has a file name the backup cannot store safely: {rel}"
            )));
        }
        let data = if is_sqlite_name(&fname) {
            sqlite_snapshot(&path, tmp_dir)?
        } else {
            std::fs::read(&path)
                .map_err(|e| AppError::Internal(format!("could not read {rel}: {e}")))?
        };
        *total += data.len();
        if *total > MAX_TOTAL_BYTES {
            return Err(AppError::BadRequest(
                "the wallet folder is too large for this backup; snapshot the whole server volume instead"
                    .into(),
            ));
        }
        out.push(Entry { name, data });
    }
    Ok(())
}

/// Write `entries` into `dest`, which must not exist or be an empty folder.
/// Never overwrites anything. Files are owner-only.
pub fn restore_to(entries: &[Entry], dest: &Path) -> Result<(), AppError> {
    if dest.exists() {
        let empty = dest.is_dir()
            && std::fs::read_dir(dest)
                .map(|mut r| r.next().is_none())
                .unwrap_or(false);
        if !empty {
            return Err(AppError::BadRequest(format!(
                "{} already exists and is not empty; restore into a new, empty folder",
                dest.display()
            )));
        }
    }
    let io = |e: std::io::Error| AppError::Internal(format!("restore failed: {e}"));
    crate::db::create_private_dir(dest).map_err(io)?;
    for e in entries {
        if !valid_name(&e.name) {
            return Err(AppError::BadRequest("unsafe file name in backup".into()));
        }
        let target = dest.join(&e.name);
        if let Some(parent) = target.parent() {
            crate::db::create_private_dir(parent).map_err(io)?;
        }
        write_private(&target, &e.data).map_err(io)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Restoring a backup onto a NEW server (setup wizard)
// ---------------------------------------------------------------------------
//
// The running server holds its database open and has its encryption key in
// memory, so a restore cannot swap them under it. The wizard therefore only
// STAGES the two files (`stage_restore`) and restarts the server; on the next
// start, before anything is opened, `apply_pending_restore` puts them in
// place. Nothing is ever deleted: the new server's own files are moved aside
// into `restore-previous-<time>/`, and the wallet folder inside the backup is
// never used (the money comes from the recovery words).

/// Why the last staged restore was not applied at this start, if it was not,
/// so the setup page can say so instead of waiting forever.
static RESTORE_REFUSED: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Record why a staged restore was refused (called once, at startup).
pub fn note_restore_refused(why: String) {
    let _ = RESTORE_REFUSED.set(why);
}

/// The reason a staged restore was refused at this start (`None` when none was).
pub fn restore_refused_reason() -> Option<&'static str> {
    RESTORE_REFUSED.get().map(String::as_str)
}

/// Folder beside the database where a staged restore waits for the next start.
pub const PENDING_DIR: &str = "restore-pending";
const READY_MARKER: &str = "ready";

/// What a staged backup holds, for the wizard to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Staged {
    pub orders: i64,
    pub stores: i64,
}

/// The folder the database lives in (and the key file, and the staging folder).
pub fn data_dir_of(database_path: &str) -> PathBuf {
    Path::new(database_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Open database bytes read-only and say what is inside. Refuses a file that
/// is damaged, is not an EBP database, or was written by a newer version.
fn check_database(bytes: &[u8], scratch_dir: &Path) -> Result<Staged, AppError> {
    let not_ours = || {
        AppError::BadRequest(
            "that backup does not hold an Easy Bitcoin Payments database; nothing was changed"
                .into(),
        )
    };
    let tmp = scratch_dir.join(format!(
        ".ebp-restore-check-{}.tmp",
        crate::util::random_hex(8)
    ));
    let result = (|| -> Result<Staged, AppError> {
        write_private(&tmp, bytes)
            .map_err(|e| AppError::Internal(format!("could not check the backup: {e}")))?;
        let conn = rusqlite::Connection::open_with_flags(
            &tmp,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|_| not_ours())?;
        let check: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|_| not_ours())?;
        if check != "ok" {
            return Err(AppError::BadRequest(
                "the database inside that backup is damaged; nothing was changed".into(),
            ));
        }
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|_| not_ours())?;
        if version > crate::db::schema_version() {
            return Err(AppError::BadRequest(
                "that backup was made by a NEWER version of Easy Bitcoin Payments than this server runs. Update this server first, then try again; nothing was changed".into(),
            ));
        }
        let count = |table: &str| -> Result<i64, AppError> {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .map_err(|_| not_ours())
        };
        Ok(Staged {
            stores: count("stores")?,
            orders: count("invoices")?,
        })
    })();
    // Our private copy (never the real database), with any side files SQLite
    // made while looking at it.
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", tmp.display()));
    }
    result
}

/// Check a backup's database and key and leave them in the staging folder
/// beside the database for the next start. Never touches the live database,
/// key or wallet. Any wallet files in the bundle are ignored on purpose.
pub fn stage_restore(entries: &[Entry], data_dir: &Path) -> Result<Staged, AppError> {
    let find = |name: &str| entries.iter().find(|e| e.name == name);
    let Some(db) = find("ebp.sqlite3") else {
        return Err(AppError::BadRequest(
            "that backup has no order history and settings inside it; nothing was changed".into(),
        ));
    };
    let Some(key) = find("ebp.key") else {
        return Err(AppError::BadRequest(
            "that backup has no encryption key inside it, so your stored secrets could not be read back; nothing was changed".into(),
        ));
    };
    let key_text = String::from_utf8(key.data.clone()).unwrap_or_default();
    crate::secrets::parse_key_hex(&key_text).map_err(|_| {
        AppError::BadRequest(
            "the encryption key inside that backup is damaged; nothing was changed".into(),
        )
    })?;
    let io = |e: std::io::Error| AppError::Internal(format!("could not stage the restore: {e}"));
    crate::db::create_private_dir(data_dir).map_err(io)?;
    let staged = check_database(&db.data, data_dir)?;

    let pending = data_dir.join(PENDING_DIR);
    if pending.exists() {
        std::fs::remove_dir_all(&pending).map_err(io)?;
    }
    crate::db::create_private_dir(&pending).map_err(io)?;
    write_private(&pending.join("ebp.sqlite3"), &db.data).map_err(io)?;
    write_private(&pending.join("ebp.key"), key_text.trim().as_bytes()).map_err(io)?;
    // Written last: a half-staged folder is never applied.
    write_private(&pending.join(READY_MARKER), b"ok").map_err(io)?;
    Ok(staged)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run once at startup, BEFORE the key or the database is opened. If a staged
/// restore is waiting, put its database and key in place and move the files
/// they replace into `restore-previous-<time>/` (never deleted). Refuses, and
/// sets the staged files aside as `restore-refused-<time>/`, when the staging
/// is incomplete or damaged, or when this server already finished setup or
/// already holds orders: a restore never overwrites a working server.
///
/// Returns a plain sentence about what happened, `Ok(None)` when nothing was
/// waiting.
pub fn apply_pending_restore(database_path: &str) -> Result<Option<String>, String> {
    if database_path == ":memory:" {
        return Ok(None);
    }
    let dir = data_dir_of(database_path);
    let pending = dir.join(PENDING_DIR);
    if !pending.is_dir() {
        return Ok(None);
    }
    let refuse = |why: String| -> Result<Option<String>, String> {
        let aside = dir.join(format!("restore-refused-{}", unix_now()));
        let _ = std::fs::rename(&pending, &aside);
        Err(format!(
            "a staged restore was NOT applied ({why}); its files were set aside in {}",
            aside.display()
        ))
    };

    let staged_db = pending.join("ebp.sqlite3");
    let staged_key = pending.join("ebp.key");
    if !pending.join(READY_MARKER).is_file() || !staged_db.is_file() || !staged_key.is_file() {
        return refuse("the staged files were incomplete".into());
    }
    let db_bytes = std::fs::read(&staged_db)
        .map_err(|e| format!("could not read the staged database: {e}"))?;
    if let Err(e) = check_database(&db_bytes, &dir) {
        return refuse(format!("the staged database failed its check: {e}"));
    }
    drop(db_bytes);

    let live_db = Path::new(database_path);
    if live_db.exists() {
        // A look at what is there now. Anything that cannot be read
        // counts as "nothing of value", but a finished setup or any order
        // means a real server: never replace that.
        let mut finished = false;
        let mut orders = 0i64;
        // (Opened read-write only so SQLite can read a leftover write-ahead
        // log; nothing is written.)
        if let Ok(conn) = rusqlite::Connection::open_with_flags(
            live_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        ) {
            finished = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'setup_complete'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map(|v| v == "true")
                .unwrap_or(false);
            orders = conn
                .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
                .unwrap_or(0);
        }
        if finished || orders > 0 {
            return refuse("this server has already finished setup or holds orders".into());
        }
    }

    // Move what is there aside, then move the staged files in. If anything
    // fails halfway, put the originals back.
    let aside = dir.join(format!("restore-previous-{}", unix_now()));
    crate::db::create_private_dir(&aside)
        .map_err(|e| format!("could not make {}: {e}", aside.display()))?;
    let wal = PathBuf::from(format!("{database_path}-wal"));
    let shm = PathBuf::from(format!("{database_path}-shm"));
    let live_key = dir.join("ebp.key");
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let undo = |moved: &[(PathBuf, PathBuf)]| {
        for (from, to) in moved.iter().rev() {
            let _ = std::fs::rename(to, from);
        }
    };
    for original in [live_db.to_path_buf(), wal, shm, live_key.clone()] {
        if !original.exists() {
            continue;
        }
        let Some(name) = original.file_name() else {
            continue;
        };
        let target = aside.join(name);
        if let Err(e) = std::fs::rename(&original, &target) {
            undo(&moved);
            return Err(format!("could not move {} aside: {e}", original.display()));
        }
        moved.push((original, target));
    }
    if let Err(e) = std::fs::rename(&staged_db, live_db) {
        undo(&moved);
        return Err(format!("could not put the restored database in place: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged_key, &live_key) {
        let _ = std::fs::rename(live_db, &staged_db);
        undo(&moved);
        return Err(format!("could not put the restored key in place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&pending);
    Ok(Some(format!(
        "restored your order history, settings and encryption key from the backup; the files they replaced are kept in {}",
        aside.display()
    )))
}

fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry {
                name: "ebp.sqlite3".into(),
                data: b"database bytes".to_vec(),
            },
            Entry {
                name: "barkd/wallet/state.db".into(),
                data: vec![0u8, 1, 2, 255],
            },
        ]
    }

    const PW: &str = "correct horse battery";

    #[test]
    fn inspect_lists_names_and_sizes_and_refuses_a_wrong_password() {
        let entries = vec![
            Entry {
                name: "ebp.sqlite3".into(),
                data: vec![1, 2, 3],
            },
            Entry {
                name: "barkd/db.sqlite".into(),
                data: vec![9; 10],
            },
        ];
        let sealed = seal_with(&entries, "a good long password", 1_000).unwrap();
        let listed = inspect(&sealed, "a good long password").unwrap();
        assert_eq!(
            listed,
            vec![
                ("ebp.sqlite3".to_string(), 3),
                ("barkd/db.sqlite".to_string(), 10)
            ]
        );
        assert!(inspect(&sealed, "the wrong password!!").is_err());
    }

    #[test]
    fn round_trip_and_nothing_readable_in_the_file() {
        let sealed = seal_with(&sample(), PW, 1_000).unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"database"));
        assert_eq!(open(&sealed, PW).unwrap(), sample());
    }

    #[test]
    fn wrong_password_fails() {
        let sealed = seal_with(&sample(), PW, 1_000).unwrap();
        assert!(open(&sealed, "another long password").is_err());
    }

    #[test]
    fn any_tampering_fails() {
        let sealed = seal_with(&sample(), PW, 1_000).unwrap();
        for i in [0usize, 9, 20, 35, HEADER_LEN + 3, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(open(&bad, PW).is_err(), "byte {i} flip was accepted");
        }
        assert!(open(&sealed[..sealed.len() - 1], PW).is_err());
        assert!(open(&sealed[..10], PW).is_err());
        assert!(open(b"", PW).is_err());
    }

    #[test]
    fn two_seals_differ() {
        let a = seal_with(&sample(), PW, 1_000).unwrap();
        let b = seal_with(&sample(), PW, 1_000).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn unsafe_names_are_refused_on_open() {
        for bad in [
            "../evil",
            "/etc/passwd",
            "a/../b",
            "a//b",
            "",
            "a b",
            "x\\y",
        ] {
            let sealed = seal_with(
                &[Entry {
                    name: bad.into(),
                    data: vec![1],
                }],
                PW,
                1_000,
            )
            .unwrap();
            assert!(open(&sealed, PW).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn password_rules() {
        assert!(validate_password("short").is_err());
        assert!(validate_password(PW).is_ok());
        assert!(validate_password(&"x".repeat(201)).is_err());
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ebp-bk-{tag}-{}", crate::util::random_hex(6)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A real EBP database file (all migrations run) with a setup flag, a
    /// store and `orders` orders, for the restore tests.
    fn real_db(path: &Path, setup_complete: bool, orders: i64, store: &str) {
        drop(crate::db::Db::open(&path.to_string_lossy()).unwrap());
        let c = rusqlite::Connection::open(path).unwrap();
        c.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('setup_complete', ?1, 0)",
            [if setup_complete { "true" } else { "false" }],
        )
        .unwrap();
        c.execute(
            "INSERT INTO stores (id, name, created_at) VALUES ('s1', ?1, 1)",
            [store],
        )
        .unwrap();
        for n in 0..orders {
            c.execute(
                "INSERT INTO invoices (id, store_id, status, amount_sats, ark_address, onchain_address, expires_at, created_at) \
                 VALUES (?1, 's1', 'paid', 1000, 'a', 'b', 9, 1)",
                [format!("inv{n}")],
            )
            .unwrap();
        }
    }

    const KEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const KEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// The entries of a backup made on an "old server": a database with 3
    /// orders, a key, and a wallet file that a restore must ignore.
    fn old_server_backup(tag: &str) -> Vec<Entry> {
        let d = tmpdir(tag);
        let db = d.join("old.sqlite3");
        real_db(&db, true, 3, "Old Shop");
        vec![
            Entry {
                name: "ebp.sqlite3".into(),
                data: std::fs::read(&db).unwrap(),
            },
            Entry {
                name: "ebp.key".into(),
                data: KEY_A.as_bytes().to_vec(),
            },
            Entry {
                name: "barkd/wallet/state.db".into(),
                data: vec![7u8; 10],
            },
        ]
    }

    #[test]
    fn a_staged_backup_replaces_a_fresh_server_and_keeps_what_it_replaced() {
        let dir = tmpdir("apply");
        let live = dir.join("ebp.sqlite3");
        // The new server's own state: first boot made a database and a key.
        real_db(&live, false, 0, "Fresh Shop");
        std::fs::write(dir.join("ebp.key"), KEY_B).unwrap();

        let staged = stage_restore(&old_server_backup("apply-src"), &dir).unwrap();
        assert_eq!(
            staged,
            Staged {
                orders: 3,
                stores: 1
            }
        );
        // Staging alone changes nothing live, and ignores the wallet files.
        assert_eq!(std::fs::read_to_string(dir.join("ebp.key")).unwrap(), KEY_B);
        assert!(!dir.join(PENDING_DIR).join("barkd").exists());

        let said = apply_pending_restore(&live.to_string_lossy())
            .unwrap()
            .unwrap();
        assert!(said.contains("restore-previous-"));

        // The old server's database and key are in place...
        assert_eq!(std::fs::read_to_string(dir.join("ebp.key")).unwrap(), KEY_A);
        let c = rusqlite::Connection::open(&live).unwrap();
        let name: String = c
            .query_row("SELECT name FROM stores", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Old Shop");
        let orders: i64 = c
            .query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orders, 3);
        // ...the staging folder is gone, and nothing was deleted.
        assert!(!dir.join(PENDING_DIR).exists());
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("restore-previous-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        let kept_key = std::fs::read_to_string(aside[0].path().join("ebp.key")).unwrap();
        assert_eq!(kept_key, KEY_B);
        assert!(aside[0].path().join("ebp.sqlite3").is_file());
        // Nothing waiting any more.
        assert_eq!(
            apply_pending_restore(&live.to_string_lossy()).unwrap(),
            None
        );
    }

    #[test]
    fn a_staged_backup_never_replaces_a_finished_or_busy_server() {
        for (complete, orders) in [(true, 0), (false, 2)] {
            let dir = tmpdir("busy");
            let live = dir.join("ebp.sqlite3");
            real_db(&live, complete, orders, "Working Shop");
            std::fs::write(dir.join("ebp.key"), KEY_B).unwrap();
            stage_restore(&old_server_backup("busy-src"), &dir).unwrap();

            let err = apply_pending_restore(&live.to_string_lossy()).unwrap_err();
            assert!(err.contains("NOT applied"), "{err}");
            // The working server is untouched and the staged files are kept aside.
            assert_eq!(std::fs::read_to_string(dir.join("ebp.key")).unwrap(), KEY_B);
            let c = rusqlite::Connection::open(&live).unwrap();
            let name: String = c
                .query_row("SELECT name FROM stores", [], |r| r.get(0))
                .unwrap();
            assert_eq!(name, "Working Shop");
            assert!(!dir.join(PENDING_DIR).exists());
            assert!(std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .any(|e| e
                    .file_name()
                    .to_string_lossy()
                    .starts_with("restore-refused-")));
        }
    }

    #[test]
    fn a_restore_onto_an_empty_data_folder_works() {
        // No database or key yet at all (nothing has booted).
        let dir = tmpdir("empty");
        let live = dir.join("ebp.sqlite3");
        stage_restore(&old_server_backup("empty-src"), &dir).unwrap();
        assert!(apply_pending_restore(&live.to_string_lossy())
            .unwrap()
            .is_some());
        assert!(live.is_file());
        assert_eq!(std::fs::read_to_string(dir.join("ebp.key")).unwrap(), KEY_A);
    }

    #[test]
    fn staging_refuses_a_backup_that_cannot_be_used() {
        let dir = tmpdir("junk");
        let good = old_server_backup("junk-src");
        let with = |name: &str, data: &[u8]| -> Vec<Entry> {
            good.iter()
                .map(|e| {
                    if e.name == name {
                        Entry {
                            name: e.name.clone(),
                            data: data.to_vec(),
                        }
                    } else {
                        e.clone()
                    }
                })
                .collect()
        };
        // Not a database; a damaged key; no database; no key.
        assert!(stage_restore(&with("ebp.sqlite3", b"not a database at all"), &dir).is_err());
        assert!(stage_restore(&with("ebp.key", b"too short"), &dir).is_err());
        let no_db: Vec<Entry> = good
            .iter()
            .filter(|e| e.name != "ebp.sqlite3")
            .cloned()
            .collect();
        assert!(stage_restore(&no_db, &dir).is_err());
        let no_key: Vec<Entry> = good
            .iter()
            .filter(|e| e.name != "ebp.key")
            .cloned()
            .collect();
        assert!(stage_restore(&no_key, &dir).is_err());
        // A refused backup leaves no staging folder behind.
        assert!(!dir.join(PENDING_DIR).exists());
    }

    #[test]
    fn a_database_from_a_newer_version_is_refused() {
        let dir = tmpdir("newer");
        let db = dir.join("newer.sqlite3");
        real_db(&db, true, 0, "Shop");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.pragma_update(None, "user_version", crate::db::schema_version() + 1)
                .unwrap();
        }
        let entries = vec![
            Entry {
                name: "ebp.sqlite3".into(),
                data: std::fs::read(&db).unwrap(),
            },
            Entry {
                name: "ebp.key".into(),
                data: KEY_A.as_bytes().to_vec(),
            },
        ];
        let err = stage_restore(&entries, &dir).unwrap_err();
        assert!(format!("{err:?}").contains("NEWER"), "{err:?}");
        assert!(!dir.join(PENDING_DIR).exists());
    }

    #[test]
    fn a_half_staged_folder_is_set_aside_not_applied() {
        let dir = tmpdir("half");
        let live = dir.join("ebp.sqlite3");
        real_db(&live, false, 0, "Fresh Shop");
        let pending = dir.join(PENDING_DIR);
        std::fs::create_dir_all(&pending).unwrap();
        std::fs::write(pending.join("ebp.sqlite3"), b"partial").unwrap();
        // No key and no ready marker: never applied.
        assert!(apply_pending_restore(&live.to_string_lossy()).is_err());
        assert!(!pending.exists());
        let c = rusqlite::Connection::open(&live).unwrap();
        let name: String = c
            .query_row("SELECT name FROM stores", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Fresh Shop");
    }

    #[test]
    fn restore_writes_private_files_and_refuses_a_busy_folder() {
        let base = tmpdir("restore");
        let dest = base.join("out");
        restore_to(&sample(), &dest).unwrap();
        assert_eq!(
            std::fs::read(dest.join("ebp.sqlite3")).unwrap(),
            b"database bytes"
        );
        assert_eq!(
            std::fs::read(dest.join("barkd/wallet/state.db")).unwrap(),
            vec![0u8, 1, 2, 255]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(dest.join("ebp.sqlite3"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o077, 0);
        }
        // Second restore into the same (now busy) folder is refused.
        assert!(restore_to(&sample(), &dest).is_err());
        // An existing EMPTY folder is fine.
        let empty = base.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        restore_to(&sample(), &empty).unwrap();
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn wallet_folder_is_snapshotted_skipping_sidecars_and_symlinks() {
        let base = tmpdir("wallet");
        let wd = base.join("barkd");
        std::fs::create_dir_all(wd.join("sub")).unwrap();
        {
            let c = rusqlite::Connection::open(wd.join("db.sqlite")).unwrap();
            c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (42);")
                .unwrap();
        }
        std::fs::write(wd.join("db.sqlite-wal"), b"junk").unwrap();
        std::fs::write(wd.join("sub/seed.bin"), b"seedfile").unwrap();
        // Lock files and the debug log are not worth backing up.
        std::fs::write(wd.join("LOCK"), b"1\n").unwrap();
        std::fs::write(wd.join("barkd.lock"), b"1\n").unwrap();
        std::fs::write(wd.join("debug.log"), b"noise").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hostname", wd.join("link")).unwrap();
        let entries = wallet_entries(&wd).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["barkd/db.sqlite", "barkd/sub/seed.bin"]);
        // The snapshot is a real, readable database.
        let out = base.join("restored");
        restore_to(&entries, &out).unwrap();
        let c = rusqlite::Connection::open(out.join("barkd/db.sqlite")).unwrap();
        let v: i64 = c.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 42);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn database_entry_drops_sessions_and_two_step_secrets() {
        let db_conn = rusqlite::Connection::open_in_memory().unwrap();
        db_conn
            .execute_batch(
                "CREATE TABLE admin_sessions(token_hash TEXT); \
                 INSERT INTO admin_sessions VALUES ('secret-session'); \
                 CREATE TABLE admin_totp(secret_enc TEXT); \
                 INSERT INTO admin_totp VALUES ('secret'); \
                 CREATE TABLE admin_recovery_codes(code_hash TEXT); \
                 INSERT INTO admin_recovery_codes VALUES ('hash'); \
                 CREATE TABLE keep(x); INSERT INTO keep VALUES (7);",
            )
            .unwrap();
        let e = database_entry(&db_conn).unwrap();
        assert_eq!(e.name, "ebp.sqlite3");
        let base = tmpdir("dbentry");
        let f = base.join("x.sqlite3");
        std::fs::write(&f, &e.data).unwrap();
        let c = rusqlite::Connection::open(&f).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM admin_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        for table in ["admin_totp", "admin_recovery_codes"] {
            let n: i64 = c
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 0, "{table} must not travel in a backup");
        }
        let k: i64 = c.query_row("SELECT x FROM keep", [], |r| r.get(0)).unwrap();
        assert_eq!(k, 7);
        let _ = std::fs::remove_dir_all(base);
    }
}
