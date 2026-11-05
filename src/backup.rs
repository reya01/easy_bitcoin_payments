//! Encrypted backup bundle (admin "Download a backup" and `ebp backup ...`).
//!
//! What a bundle holds:
//! * `ebp.sqlite3` — a consistent snapshot of the EBP database (invoices,
//!   stores, payment history, settings). Signed-in admin sessions are
//!   removed from the copy. Secrets inside it (webhook secrets) stay
//!   encrypted with the server's key-encryption key, which is NOT in the
//!   bundle: keep that key separately.
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

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

use crate::error::AppError;

const MAGIC: &[u8; 8] = b"EBPBAK1\n";
const HEADER_LEN: usize = 8 + 4 + 16 + 12;
pub const DEFAULT_ITERATIONS: u32 = 600_000;
const MIN_ITERATIONS: u32 = 1_000;
const MAX_ITERATIONS: u32 = 5_000_000;
pub const MIN_PASSWORD_LEN: usize = 12;
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

fn cipher(password: &str, salt: &[u8], iterations: u32) -> ChaCha20Poly1305 {
    let key = crate::admin_auth::pbkdf2(password.as_bytes(), salt, iterations);
    ChaCha20Poly1305::new(Key::from_slice(&key))
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
    let ct = cipher(password, &salt, iterations)
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &out[..HEADER_LEN],
            },
        )
        .map_err(|_| AppError::Internal("could not encrypt the backup".into()))?;
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt and parse a bundle. A wrong password and a damaged file look the
/// same on purpose (the cipher cannot tell them apart).
pub fn open(bytes: &[u8], password: &str) -> Result<Vec<Entry>, AppError> {
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
    let plain = cipher(password, salt, iterations)
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &bytes[HEADER_LEN..],
                aad: &bytes[..HEADER_LEN],
            },
        )
        .map_err(|_| {
            AppError::BadRequest(
                "could not open the backup: wrong backup password, or the file is damaged".into(),
            )
        })?;
    decode(&plain)
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
pub fn database_entry(conn: &rusqlite::Connection, database_path: &str) -> Result<Entry, AppError> {
    let dir: PathBuf = if database_path == ":memory:" {
        std::env::temp_dir()
    } else {
        Path::new(database_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let tmp = dir.join(format!(".ebp-backup-{}.tmp", crate::util::random_hex(8)));
    let result = (|| -> Result<Vec<u8>, AppError> {
        conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])?;
        {
            let copy = rusqlite::Connection::open(&tmp)?;
            // A restored copy must not carry live logins.
            copy.execute("DELETE FROM admin_sessions", [])?;
        }
        std::fs::read(&tmp).map_err(|e| AppError::Internal(format!("backup read failed: {e}")))
    })();
    let _ = std::fs::remove_file(&tmp);
    Ok(Entry {
        name: "ebp.sqlite3".into(),
        data: result?,
    })
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
        if !ft.is_file() || is_sqlite_sidecar(&fname) {
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
    fn database_entry_drops_sessions() {
        let db_conn = rusqlite::Connection::open_in_memory().unwrap();
        db_conn
            .execute_batch(
                "CREATE TABLE admin_sessions(token_hash TEXT); \
                 INSERT INTO admin_sessions VALUES ('secret-session'); \
                 CREATE TABLE keep(x); INSERT INTO keep VALUES (7);",
            )
            .unwrap();
        let e = database_entry(&db_conn, ":memory:").unwrap();
        assert_eq!(e.name, "ebp.sqlite3");
        let base = tmpdir("dbentry");
        let f = base.join("x.sqlite3");
        std::fs::write(&f, &e.data).unwrap();
        let c = rusqlite::Connection::open(&f).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM admin_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let k: i64 = c.query_row("SELECT x FROM keep", [], |r| r.get(0)).unwrap();
        assert_eq!(k, 7);
        let _ = std::fs::remove_dir_all(base);
    }
}
