//! Two-step sign-in with an authenticator app (TOTP, RFC 6238).
//!
//! What it does and why: signing in needs the admin password AND a 6-digit code
//! from an app on the owner's phone, so a stolen or guessed password alone is not
//! enough to get in or to change where the money goes.
//!
//! How (background detail): HMAC-SHA1 with 30-second steps and 6 digits, the
//! settings every authenticator app supports. The shared secret is stored
//! encrypted (`secrets`), recovery codes are stored hashed, a code cannot be used
//! twice, and five wrong codes lock further tries for ten minutes. HMAC-SHA1 is
//! what the standard specifies for this use; it is not used for anything else.

use rand::RngCore;
use ring::hmac;
use rusqlite::{params, Connection, OptionalExtension};

use crate::auth::{ct_eq, hash_secret};
use crate::error::AppError;
use crate::util::now_unix;

const STEP_SECS: i64 = 30;
const DIGITS: u32 = 6;
const SECRET_LEN: usize = 20;
const MAX_FAILS: i64 = 5;
const LOCK_SECS: i64 = 600;
pub const RECOVERY_CODES: usize = 10;
const AAD: &str = "admin_totp";
const DECLINED_KEY: &str = "twofa_declined";

const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32 without padding (what authenticator apps expect).
pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(B32[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(B32[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Decode base32, ignoring spaces, dashes, padding and letter case.
pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.chars() {
        if c == ' ' || c == '-' || c == '=' {
            continue;
        }
        let v = B32
            .iter()
            .position(|&x| x as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push(((buf >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// RFC 4226 HOTP value for one counter, as a number below 10^6.
pub fn hotp(key: &[u8], counter: u64) -> u32 {
    let k = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
    let tag = hmac::sign(&k, &counter.to_be_bytes());
    let h = tag.as_ref();
    let off = (h[19] & 0x0f) as usize;
    let bin = ((h[off] as u32 & 0x7f) << 24)
        | ((h[off + 1] as u32) << 16)
        | ((h[off + 2] as u32) << 8)
        | (h[off + 3] as u32);
    bin % 10u32.pow(DIGITS)
}

/// The 6-digit code for a 30-second step number.
pub fn code_for_step(key: &[u8], step: i64) -> String {
    format!("{:06}", hotp(key, step as u64))
}

/// The step a typed code belongs to, if it matches the current step or one
/// either side (clock drift), and is newer than `min_step`.
pub fn matching_step(key: &[u8], typed: &str, now: i64, min_step: i64) -> Option<i64> {
    let typed: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    if typed.len() != DIGITS as usize || !typed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let cur = now / STEP_SECS;
    let mut hit = None;
    for step in [cur - 1, cur, cur + 1] {
        // Check every candidate (no early exit) so timing does not reveal which.
        if ct_eq(&code_for_step(key, step), &typed) && step >= min_step {
            hit = Some(step);
        }
    }
    hit
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `otpauth://` link inside the QR code that authenticator apps scan.
pub fn otpauth_uri(secret_b32: &str, account: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={secret_b32}&issuer={}&algorithm=SHA1&digits=6&period=30",
        percent_encode("Easy Bitcoin Payments"),
        percent_encode(account),
        percent_encode("Easy Bitcoin Payments")
    )
}

/// Result of checking the second step.
#[derive(Debug, PartialEq, Eq)]
pub enum Second {
    Ok,
    Wrong,
    Locked,
}

struct Row {
    secret_enc: String,
    enabled: bool,
    last_step: i64,
    fail_count: i64,
    locked_until: i64,
}

fn row(conn: &Connection) -> Result<Option<Row>, AppError> {
    Ok(conn
        .query_row(
            "SELECT secret_enc, enabled, last_step, fail_count, locked_until
             FROM admin_totp WHERE id = 1",
            [],
            |r| {
                Ok(Row {
                    secret_enc: r.get(0)?,
                    enabled: r.get::<_, i64>(1)? != 0,
                    last_step: r.get(2)?,
                    fail_count: r.get(3)?,
                    locked_until: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// Is two-step sign-in turned on (confirmed)?
pub fn is_enabled(conn: &Connection) -> Result<bool, AppError> {
    Ok(row(conn)?.map(|r| r.enabled).unwrap_or(false))
}

/// Did the owner choose "password only"?
pub fn is_declined(conn: &Connection) -> Result<bool, AppError> {
    crate::settings::get_bool(conn, DECLINED_KEY)
}

pub fn set_declined(conn: &Connection, declined: bool) -> Result<(), AppError> {
    crate::settings::set(conn, DECLINED_KEY, if declined { "true" } else { "false" })
}

pub fn recovery_left(conn: &Connection) -> Result<i64, AppError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM admin_recovery_codes WHERE used_at IS NULL",
        [],
        |r| r.get(0),
    )?)
}

/// Start turning it on: make a new secret (not active until a code confirms it).
/// Returns the secret as text the owner can type if they cannot scan.
pub fn begin_setup(conn: &Connection) -> Result<String, AppError> {
    if is_enabled(conn)? {
        return Err(AppError::Conflict(
            "Two-step sign-in is already on. Turn it off first to set it up again.".into(),
        ));
    }
    let mut secret = [0u8; SECRET_LEN];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let b32 = base32_encode(&secret);
    let enc = crate::secrets::get().encrypt(&b32, AAD)?;
    conn.execute(
        "INSERT INTO admin_totp (id, secret_enc, enabled, last_step, fail_count, locked_until, created_at)
         VALUES (1, ?1, 0, 0, 0, 0, ?2)
         ON CONFLICT(id) DO UPDATE SET secret_enc = excluded.secret_enc, enabled = 0,
            last_step = 0, fail_count = 0, locked_until = 0, created_at = excluded.created_at",
        params![enc, now_unix()],
    )?;
    Ok(b32)
}

fn key_of(enc: &str) -> Result<Vec<u8>, AppError> {
    let b32 = crate::secrets::get().decrypt(enc, AAD)?;
    base32_decode(&b32).ok_or_else(|| AppError::Internal("stored sign-in secret is damaged".into()))
}

fn note_failure(conn: &Connection, fails: i64, now: i64) -> Result<Second, AppError> {
    let fails = fails + 1;
    if fails >= MAX_FAILS {
        conn.execute(
            "UPDATE admin_totp SET fail_count = 0, locked_until = ?1 WHERE id = 1",
            params![now + LOCK_SECS],
        )?;
        Ok(Second::Locked)
    } else {
        conn.execute(
            "UPDATE admin_totp SET fail_count = ?1 WHERE id = 1",
            params![fails],
        )?;
        Ok(Second::Wrong)
    }
}

fn normalise_recovery(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn generate_recovery_codes(conn: &Connection) -> Result<Vec<String>, AppError> {
    conn.execute("DELETE FROM admin_recovery_codes", [])?;
    let mut codes = Vec::new();
    for _ in 0..RECOVERY_CODES {
        let mut raw = [0u8; 7];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let s = base32_encode(&raw).to_ascii_lowercase();
        let code = format!("{}-{}", &s[..5], &s[5..10]);
        conn.execute(
            "INSERT OR IGNORE INTO admin_recovery_codes (code_hash, used_at) VALUES (?1, NULL)",
            params![hash_secret(&normalise_recovery(&code))],
        )?;
        codes.push(code);
    }
    Ok(codes)
}

/// Check a typed value (6-digit code, or a recovery code) against the stored
/// secret. `allow_same_step` lets one code serve a sign-in and an action right
/// after it (used for sensitive actions); sign-in itself never re-uses a code.
pub fn verify(
    conn: &Connection,
    typed: &str,
    now: i64,
    allow_same_step: bool,
) -> Result<Second, AppError> {
    let Some(Row {
        secret_enc: enc,
        enabled,
        last_step,
        fail_count: fails,
        locked_until,
    }) = row(conn)?
    else {
        return Ok(Second::Wrong);
    };
    if !enabled {
        return Ok(Second::Wrong);
    }
    if locked_until > now {
        return Ok(Second::Locked);
    }
    let cleaned: String = typed.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() == DIGITS as usize && cleaned.chars().all(|c| c.is_ascii_digit()) {
        let key = key_of(&enc)?;
        let min = if allow_same_step {
            last_step
        } else {
            last_step + 1
        };
        if let Some(step) = matching_step(&key, &cleaned, now, min) {
            conn.execute(
                "UPDATE admin_totp SET last_step = MAX(last_step, ?1), fail_count = 0 WHERE id = 1",
                params![step],
            )?;
            return Ok(Second::Ok);
        }
        return note_failure(conn, fails, now);
    }
    // A recovery code: works once.
    let norm = normalise_recovery(typed);
    if norm.len() == 10 {
        let n = conn.execute(
            "UPDATE admin_recovery_codes SET used_at = ?1 WHERE code_hash = ?2 AND used_at IS NULL",
            params![now, hash_secret(&norm)],
        )?;
        if n == 1 {
            conn.execute("UPDATE admin_totp SET fail_count = 0 WHERE id = 1", [])?;
            return Ok(Second::Ok);
        }
    }
    note_failure(conn, fails, now)
}

/// Finish turning it on: the owner typed a code from their app. Returns the
/// recovery codes (shown once).
pub fn confirm(conn: &Connection, typed: &str, now: i64) -> Result<Vec<String>, AppError> {
    let Some(Row {
        secret_enc: enc,
        enabled,
        fail_count: fails,
        locked_until,
        ..
    }) = row(conn)?
    else {
        return Err(AppError::BadRequest(
            "Start again: press \"Turn on\" to get a new code to scan.".into(),
        ));
    };
    if enabled {
        return Err(AppError::Conflict("Two-step sign-in is already on.".into()));
    }
    if locked_until > now {
        return Err(AppError::BadRequest(
            "Too many wrong codes. Wait 10 minutes and try again.".into(),
        ));
    }
    let key = key_of(&enc)?;
    match matching_step(&key, typed, now, 0) {
        Some(step) => {
            conn.execute(
                "UPDATE admin_totp SET enabled = 1, last_step = ?1, fail_count = 0, locked_until = 0
                 WHERE id = 1",
                params![step],
            )?;
            set_declined(conn, false)?;
            generate_recovery_codes(conn)
        }
        None => {
            let _ = note_failure(conn, fails, now)?;
            Err(AppError::BadRequest(
                "That code did not match. Check the 6-digit code in your app (it changes every 30 seconds) and try again.".into(),
            ))
        }
    }
}

/// Turn it off and forget the secret and recovery codes.
pub fn disable(conn: &Connection) -> Result<(), AppError> {
    conn.execute("DELETE FROM admin_totp", [])?;
    conn.execute("DELETE FROM admin_recovery_codes", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC_KEY: &[u8] = b"12345678901234567890";

    #[test]
    fn hotp_matches_rfc_4226_vectors() {
        let want = [
            755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
        ];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(hotp(RFC_KEY, i as u64), *w, "counter {i}");
        }
    }

    #[test]
    fn totp_matches_rfc_6238_sha1_vectors() {
        // RFC 6238 gives 8-digit values; the last 6 digits are ours.
        for (t, want) in [
            (59i64, "287082"),
            (1111111109, "081804"),
            (1234567890, "005924"),
        ] {
            assert_eq!(code_for_step(RFC_KEY, t / 30), want, "time {t}");
        }
    }

    #[test]
    fn base32_round_trips_and_matches_rfc_4648() {
        assert_eq!(base32_encode(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32_decode("mzxw 6ytb-oi").unwrap(), b"foobar");
        assert!(base32_decode("not base32!").is_none());
        let secret = [7u8; 20];
        assert_eq!(base32_decode(&base32_encode(&secret)).unwrap(), secret);
    }

    #[test]
    fn matching_step_allows_drift_but_not_replay() {
        let now = 1_700_000_000;
        let cur = now / 30;
        let code = code_for_step(RFC_KEY, cur);
        assert_eq!(matching_step(RFC_KEY, &code, now, 0), Some(cur));
        // one step early or late still works
        assert_eq!(
            matching_step(RFC_KEY, &code_for_step(RFC_KEY, cur - 1), now, 0),
            Some(cur - 1)
        );
        assert_eq!(
            matching_step(RFC_KEY, &code_for_step(RFC_KEY, cur + 1), now, 0),
            Some(cur + 1)
        );
        // two steps away does not
        assert_eq!(
            matching_step(RFC_KEY, &code_for_step(RFC_KEY, cur - 2), now, 0),
            None
        );
        // already-used step is refused
        assert_eq!(matching_step(RFC_KEY, &code, now, cur + 1), None);
        // wrong shape is refused
        assert_eq!(matching_step(RFC_KEY, "12345", now, 0), None);
        assert_eq!(matching_step(RFC_KEY, "12345a", now, 0), None);
        // spaces are tolerated ("123 456")
        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert_eq!(matching_step(RFC_KEY, &spaced, now, 0), Some(cur));
    }

    #[test]
    fn otpauth_link_is_escaped() {
        let u = otpauth_uri("ABCDEF", "My Shop & Co");
        assert!(u.starts_with("otpauth://totp/Easy%20Bitcoin%20Payments:My%20Shop%20%26%20Co?"));
        assert!(u.contains("secret=ABCDEF"));
        assert!(u.contains("digits=6") && u.contains("period=30"));
    }
}
