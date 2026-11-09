//! Merchant admin login: password hashing, sessions, cookies.
//!
//! Threat model: the admin page is reachable from the internet. The only way
//! to CREATE the password is the setup wizard (protected by the one-time setup
//! code) or the `ebp admin set-password` command on the server, so a stranger
//! who finds an installed server cannot claim it. Login is rate limited, the
//! hash is slow (PBKDF2-HMAC-SHA256, 600k rounds by default, OWASP guidance),
//! the session cookie is HttpOnly + SameSite=Strict (+ Secure on https), the
//! server stores only a hash of the session token, and every state-changing
//! request must carry the session's CSRF token in a header.

use axum::http::{header, HeaderMap};
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::Sha256;

use crate::auth::{ct_eq, hash_secret};
use crate::error::AppError;
use crate::util::{now_unix, random_hex};

/// Session cookie name. Over HTTPS it carries the `__Host-` prefix, which makes
/// the browser refuse any copy that is not Secure, host-only and Path=/ (so a
/// sibling subdomain cannot plant a session cookie). Plain http (localhost
/// testing) cannot use the prefix.
pub const COOKIE_NAME: &str = "ebp_session";
pub const COOKIE_NAME_SECURE: &str = "__Host-ebp_session";

pub fn cookie_name(secure: bool) -> &'static str {
    if secure {
        COOKIE_NAME_SECURE
    } else {
        COOKIE_NAME
    }
}

/// Is the public address https? Decides the cookie name and the Secure flag.
pub fn is_secure(public_base_url: &str) -> bool {
    public_base_url
        .trim()
        .to_ascii_lowercase()
        .starts_with("https://")
}
pub const CSRF_HEADER: &str = "x-ebp-csrf";
pub const DEFAULT_ITERATIONS: u32 = 600_000;
pub const MIN_ITERATIONS: u32 = 1_000;
pub const MIN_PASSWORD_LEN: usize = 16;
pub const MAX_PASSWORD_LEN: usize = 200;
const SESSION_ABSOLUTE_SECS: i64 = 7 * 86_400;
const SESSION_IDLE_SECS: i64 = 12 * 3_600;

type HmacSha256 = Hmac<Sha256>;

pub fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let base = HmacSha256::new_from_slice(password).expect("hmac accepts any key length");
    let mut first = base.clone();
    first.update(salt);
    first.update(&1u32.to_be_bytes());
    let mut u: [u8; 32] = first.finalize().into_bytes().into();
    let mut t = u;
    for _ in 1..iterations {
        let mut m = base.clone();
        m.update(&u);
        u = m.finalize().into_bytes().into();
        for (a, b) in t.iter_mut().zip(u.iter()) {
            *a ^= b;
        }
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&t);
    out
}

pub fn validate_password(password: &str) -> Result<(), AppError> {
    let n = password.chars().count();
    if n < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "choose a password of at least {MIN_PASSWORD_LEN} characters (a few random words works well)"
        )));
    }
    if n > MAX_PASSWORD_LEN {
        return Err(AppError::BadRequest("that password is too long".into()));
    }
    Ok(())
}

/// `pbkdf2-sha256$<iterations>$<salt hex>$<hash hex>`
pub fn hash_password(password: &str, iterations: u32) -> String {
    let iterations = iterations.max(MIN_ITERATIONS);
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let h = pbkdf2(password.as_bytes(), &salt, iterations);
    format!(
        "pbkdf2-sha256${iterations}${}${}",
        hex::encode(salt),
        hex::encode(h)
    )
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let [scheme, iters, salt, hash] = parts.as_slice() else {
        return false;
    };
    if *scheme != "pbkdf2-sha256" {
        return false;
    }
    let (Ok(iters), Ok(salt)) = (iters.parse::<u32>(), hex::decode(salt)) else {
        return false;
    };
    if iters < MIN_ITERATIONS {
        return false;
    }
    let h = pbkdf2(password.as_bytes(), &salt, iters);
    ct_eq(&hex::encode(h), hash)
}

pub fn password_is_set(conn: &Connection) -> Result<bool, AppError> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM admin_account", [], |r| r.get(0))?;
    Ok(n > 0)
}

/// Create or replace the admin password. Replacing signs out every session.
pub fn set_password(conn: &Connection, password: &str, iterations: u32) -> Result<(), AppError> {
    validate_password(password)?;
    let hash = hash_password(password, iterations);
    conn.execute(
        "INSERT INTO admin_account (id, password_hash, updated_at) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET password_hash = excluded.password_hash,
                                       updated_at = excluded.updated_at",
        params![hash, now_unix()],
    )?;
    conn.execute("DELETE FROM admin_sessions", [])?;
    Ok(())
}

/// The stored password hash, if a password has been set. Cheap: callers read
/// this inside the database lock, then run the SLOW check outside it with
/// [`verify_login`] so a flood of login attempts cannot stall the database.
pub fn stored_hash(conn: &Connection) -> Result<Option<String>, AppError> {
    Ok(conn
        .query_row(
            "SELECT password_hash FROM admin_account WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?)
}

/// The slow part of a login check (PBKDF2). Does the work even when no
/// password is set, so timing does not reveal that. Pure CPU: run it on a
/// blocking thread, never while holding the database.
pub fn verify_login(stored: Option<&str>, password: &str) -> bool {
    match stored {
        Some(s) => verify_password(password, s),
        None => {
            let _ = pbkdf2(password.as_bytes(), b"ebp-no-account", DEFAULT_ITERATIONS);
            false
        }
    }
}

/// At most this many slow password checks (PBKDF2) run at once. Each takes
/// a good fraction of a second of CPU on purpose; without a limit a flood of
/// guesses could keep a one-CPU server busy and slow down checkout.
const SLOW_CHECK_SLOTS: usize = 2;
/// How long a check waits for a free slot before giving up.
const SLOW_CHECK_WAIT_SECS: u64 = 10;
static SLOW_CHECKS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(SLOW_CHECK_SLOTS)));

/// Run a slow password-derived-key job (a login check, opening a backup) on a
/// blocking thread, with at most [`SLOW_CHECK_SLOTS`] at once.
pub async fn run_slow<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, AppError> {
    let permit = tokio::time::timeout(
        std::time::Duration::from_secs(SLOW_CHECK_WAIT_SECS),
        SLOW_CHECKS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| {
        AppError::TooManyRequests(
            "The server is busy checking passwords. Wait a moment and try again.".into(),
        )
    })?
    .map_err(|_| AppError::Internal("password check unavailable".into()))?;
    // The slot travels with the work: if the caller hangs up, it is still
    // held until the check really finishes, so hang-ups cannot stack up
    // more than the allowed number of checks.
    tokio::task::spawn_blocking(move || {
        let _slot = permit;
        job()
    })
    .await
    .map_err(|_| AppError::Internal("password check failed".into()))
}

/// [`verify_login`] through [`run_slow`]: what every route uses.
pub async fn verify_login_slow(stored: Option<String>, password: String) -> Result<bool, AppError> {
    run_slow(move || verify_login(stored.as_deref(), &password)).await
}

/// Check a login attempt (hash read and verified together). Convenience for
/// tests and low-traffic callers; the login route uses the split form.
pub fn check_password(conn: &Connection, password: &str) -> Result<bool, AppError> {
    let stored = stored_hash(conn)?;
    Ok(verify_login(stored.as_deref(), password))
}

pub struct Session {
    pub token: String,
    pub csrf: String,
}

pub fn create_session(conn: &Connection) -> Result<Session, AppError> {
    let token = random_hex(32);
    let csrf = random_hex(16);
    let now = now_unix();
    // Housekeeping: drop dead sessions.
    conn.execute(
        "DELETE FROM admin_sessions WHERE expires_at <= ?1 OR last_seen <= ?2",
        params![now, now - SESSION_IDLE_SECS],
    )?;
    conn.execute(
        "INSERT INTO admin_sessions (token_hash, csrf, created_at, expires_at, last_seen)
         VALUES (?1, ?2, ?3, ?4, ?3)",
        params![hash_secret(&token), csrf, now, now + SESSION_ABSOLUTE_SECS],
    )?;
    Ok(Session { token, csrf })
}

/// The CSRF token of a live session, refreshing its idle timer.
pub fn lookup_session(conn: &Connection, token: &str) -> Result<Option<String>, AppError> {
    let now = now_unix();
    let h = hash_secret(token);
    let csrf: Option<String> = conn
        .query_row(
            "SELECT csrf FROM admin_sessions
             WHERE token_hash = ?1 AND expires_at > ?2 AND last_seen > ?3",
            params![h, now, now - SESSION_IDLE_SECS],
            |r| r.get(0),
        )
        .optional()?;
    if csrf.is_some() {
        conn.execute(
            "UPDATE admin_sessions SET last_seen = ?1 WHERE token_hash = ?2",
            params![now, h],
        )?;
    }
    Ok(csrf)
}

pub fn delete_session(conn: &Connection, token: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM admin_sessions WHERE token_hash = ?1",
        params![hash_secret(token)],
    )?;
    Ok(())
}

/// The session token from the request's Cookie header, if any.
pub fn token_from_headers(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = cookie_name(secure);
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (k, v) = part.trim().split_once('=')?;
        (k == name && !v.is_empty()).then(|| v.to_string())
    })
}

pub fn set_cookie_value(token: &str, secure: bool) -> String {
    format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_ABSOLUTE_SECS}{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie_value(secure: bool) -> String {
    format!(
        "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        cookie_name(secure),
        if secure { "; Secure" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hexs(b: &[u8]) -> String {
        hex::encode(b)
    }

    #[test]
    fn pbkdf2_matches_known_vectors() {
        assert_eq!(
            hexs(&pbkdf2(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hexs(&pbkdf2(b"password", b"salt", 2)),
            "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
    }

    #[test]
    fn hash_and_verify_round_trip() {
        let h = hash_password("correct horse battery", 1_000);
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("correct horse batterz", &h));
        assert!(!verify_password("x", "garbage"));
        assert!(
            !verify_password("x", "pbkdf2-sha256$1$aa$bb"),
            "iteration floor"
        );
    }

    #[test]
    fn password_rules() {
        assert!(validate_password("short").is_err());
        // 15 is too short, 16 is enough (a few random words easily clears it).
        assert!(validate_password("fifteen chars!!").is_err());
        assert!(validate_password("sixteen chars!!!").is_ok());
        assert!(validate_password("long enough password").is_ok());
        assert!(validate_password(&"a".repeat(201)).is_err());
    }

    #[test]
    fn sessions_work_and_die() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../migrations/0009_admin_login.sql"))
            .unwrap();
        set_password(&conn, "a perfectly fine password", 1_000).unwrap();
        assert!(check_password(&conn, "a perfectly fine password").unwrap());
        assert!(!check_password(&conn, "nope nope nope nope").unwrap());
        let s = create_session(&conn).unwrap();
        assert_eq!(
            lookup_session(&conn, &s.token).unwrap(),
            Some(s.csrf.clone())
        );
        assert_eq!(lookup_session(&conn, "bogus").unwrap(), None);
        // A new password signs everything out.
        set_password(&conn, "another fine password", 1_000).unwrap();
        assert_eq!(lookup_session(&conn, &s.token).unwrap(), None);
        let s = create_session(&conn).unwrap();
        delete_session(&conn, &s.token).unwrap();
        assert_eq!(lookup_session(&conn, &s.token).unwrap(), None);
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            "a=1; ebp_session=abc123; b=2".parse().unwrap(),
        );
        assert_eq!(token_from_headers(&h, false).as_deref(), Some("abc123"));
        // Over https only the __Host- cookie counts: a planted plain one is ignored.
        assert_eq!(token_from_headers(&h, true), None);
        h.insert(
            header::COOKIE,
            "ebp_session=planted; __Host-ebp_session=real"
                .parse()
                .unwrap(),
        );
        assert_eq!(token_from_headers(&h, true).as_deref(), Some("real"));
        let c = set_cookie_value("t", true);
        assert!(
            c.starts_with("__Host-ebp_session=t;") && c.contains("Secure") && c.contains("Path=/")
        );
        assert!(!c.contains("Domain"));
        assert!(clear_cookie_value(true).starts_with("__Host-ebp_session=;"));
        assert!(set_cookie_value("t", false).starts_with("ebp_session=t;"));
        assert!(set_cookie_value("t", true).contains("Secure"));
        assert!(set_cookie_value("t", false).contains("SameSite=Strict"));
    }
}
