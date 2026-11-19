//! Two-step sign-in with an authenticator app (TOTP, RFC 6238).
//!
//! What it does and why: signing in needs the admin password AND a 6-digit code
//! from an app on the owner's phone, so a stolen or guessed password alone is not
//! enough to get in or to change where the money goes.
//!
//! How (background detail): HMAC-SHA1 with 30-second steps and 6 digits, the
//! settings every authenticator app supports. The shared secret is stored
//! encrypted (`secrets`), recovery codes are stored hashed, a code cannot be used
//! twice, and three wrong codes lock further tries (for ten minutes the first
//! time, then an hour, then six hours if it keeps happening; a good code starts
//! again at ten minutes). A code that has approved one protected action cannot
//! approve a second one. HMAC-SHA1 is what the standard specifies for this use;
//! it is not used for anything else.

use rand::RngCore;
use ring::hmac;
use rusqlite::{params, Connection, OptionalExtension};

use crate::auth::{ct_eq, hash_secret};
use crate::error::AppError;
use crate::util::now_unix;

const STEP_SECS: i64 = 30;
const DIGITS: u32 = 6;
const SECRET_LEN: usize = 20;
/// Wrong codes in a row before further tries are locked.
pub const MAX_FAILS: i64 = 3;
/// How long three wrong codes lock further tries, by how many times in a row it
/// has happened: ten minutes, an hour, then six hours at most.
const LOCK_LADDER: [i64; 3] = [600, 3_600, 21_600];
/// A lock last started longer ago than this no longer counts towards the next.
const LOCK_FORGET_SECS: i64 = 86_400;
const LOCK_LEVEL_KEY: &str = "twofa_lock_level";
const LOCK_LEVEL_AT_KEY: &str = "twofa_lock_level_at";
/// The newest 30-second step whose code has approved a protected action.
const ACTION_STEP_KEY: &str = "twofa_action_step";
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
    /// A correct code that already approved a protected action. Not a wrong
    /// code: nothing is counted against the owner, who just waits for the next
    /// code in their app.
    Spent,
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
    forget_lock_level(conn)?;
    conn.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![ACTION_STEP_KEY],
    )?;
    Ok(b32)
}

fn key_of(enc: &str) -> Result<Vec<u8>, AppError> {
    let b32 = crate::secrets::get().decrypt(enc, AAD)?;
    base32_decode(&b32).ok_or_else(|| AppError::Internal("stored sign-in secret is damaged".into()))
}

fn setting_number(conn: &Connection, key: &str) -> Result<i64, AppError> {
    Ok(crate::settings::get(conn, key)?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0))
}

/// How long the lock that starts now lasts, and remember that it happened so
/// the next one (soon after) is longer.
fn next_lock_secs(conn: &Connection, now: i64) -> Result<i64, AppError> {
    let level = if now - setting_number(conn, LOCK_LEVEL_AT_KEY)? > LOCK_FORGET_SECS {
        0
    } else {
        setting_number(conn, LOCK_LEVEL_KEY)?.max(0)
    };
    let rung = (level as usize).min(LOCK_LADDER.len() - 1);
    crate::settings::set(conn, LOCK_LEVEL_KEY, &(level + 1).to_string())?;
    crate::settings::set(conn, LOCK_LEVEL_AT_KEY, &now.to_string())?;
    Ok(LOCK_LADDER[rung])
}

/// A good code, or a fresh setup, starts the lock ladder again at ten minutes.
fn forget_lock_level(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM settings WHERE key IN (?1, ?2)",
        params![LOCK_LEVEL_KEY, LOCK_LEVEL_AT_KEY],
    )?;
    Ok(())
}

/// Seconds left on the current lock (0 when not locked).
pub fn lock_remaining_secs(conn: &Connection, now: i64) -> Result<i64, AppError> {
    Ok(row(conn)?
        .map(|r| (r.locked_until - now).max(0))
        .unwrap_or(0))
}

/// "10 minutes", "about 2 hours": a wait in words an owner can act on.
pub fn wait_text(secs: i64) -> String {
    let minutes = (secs.max(0) + 59) / 60;
    if minutes <= 1 {
        "a minute".to_string()
    } else if minutes < 90 {
        format!("{minutes} minutes")
    } else {
        format!("about {} hours", (minutes + 59) / 60)
    }
}

/// What to tell the owner while the second step is locked.
pub fn locked_message(conn: &Connection, now: i64) -> Result<String, AppError> {
    let wait = lock_remaining_secs(conn, now)?;
    let mut msg = format!("Too many wrong codes. Try again in {}.", wait_text(wait));
    if wait >= 3_000 {
        msg.push_str(" If you cannot wait, the guide \"If something looks wrong\" shows how to turn two-step sign-in off from your server's console.");
    }
    Ok(msg)
}

fn note_failure(conn: &Connection, fails: i64, now: i64) -> Result<Second, AppError> {
    let fails = fails + 1;
    if fails >= MAX_FAILS {
        let lock = next_lock_secs(conn, now)?;
        conn.execute(
            "UPDATE admin_totp SET fail_count = 0, locked_until = ?1 WHERE id = 1",
            params![now + lock],
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
/// secret. `allow_same_step` is for protected actions: the code that just signed
/// the owner in may approve ONE action right after it, but a code that has
/// approved an action is spent (the next one needs the next code from the app).
/// Sign-in itself never re-uses a code.
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
            // A code that already approved an action is spent.
            last_step.max(setting_number(conn, ACTION_STEP_KEY)? + 1)
        } else {
            last_step + 1
        };
        if let Some(step) = matching_step(&key, &cleaned, now, min) {
            conn.execute(
                "UPDATE admin_totp SET last_step = MAX(last_step, ?1), fail_count = 0 WHERE id = 1",
                params![step],
            )?;
            if allow_same_step {
                crate::settings::set(conn, ACTION_STEP_KEY, &step.to_string())?;
            }
            forget_lock_level(conn)?;
            return Ok(Second::Ok);
        }
        // A correct code that was only used up: say so, count nothing.
        if allow_same_step && matching_step(&key, &cleaned, now, last_step).is_some() {
            return Ok(Second::Spent);
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
            forget_lock_level(conn)?;
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
        return Err(AppError::BadRequest(locked_message(conn, now)?));
    }
    let key = key_of(&enc)?;
    match matching_step(&key, typed, now, 0) {
        Some(step) => {
            // All or nothing: never "on" without the recovery codes made.
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE admin_totp SET enabled = 1, last_step = ?1, fail_count = 0, locked_until = 0
                 WHERE id = 1",
                params![step],
            )?;
            set_declined(&tx, false)?;
            let codes = generate_recovery_codes(&tx)?;
            forget_lock_level(&tx)?;
            tx.execute(
                "DELETE FROM settings WHERE key = ?1",
                params![ACTION_STEP_KEY],
            )?;
            tx.commit()?;
            Ok(codes)
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
    forget_lock_level(conn)?;
    conn.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![ACTION_STEP_KEY],
    )?;
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

    use crate::db::Db;

    /// A database with two-step sign-in switched on at `now`, and its key.
    async fn with_two_step(now: i64) -> (Db, Vec<u8>) {
        let db = Db::open(":memory:").unwrap();
        let key = db
            .call(move |c| {
                let b32 = begin_setup(c)?;
                let key = base32_decode(&b32).unwrap();
                confirm(c, &code_for_step(&key, now / STEP_SECS), now)?;
                Ok(key)
            })
            .await
            .unwrap();
        (db, key)
    }

    /// A 6-digit code that is not valid at `now`.
    fn wrong_code(key: &[u8], now: i64) -> String {
        let cur = now / STEP_SECS;
        let valid: Vec<String> = (cur - 1..=cur + 1).map(|s| code_for_step(key, s)).collect();
        (0..1_000_000u32)
            .map(|n| format!("{n:06}"))
            .find(|c| !valid.contains(c))
            .unwrap()
    }

    #[tokio::test]
    async fn repeated_lockouts_get_longer_and_a_good_code_starts_again() {
        let now0 = 1_700_000_000;
        let (db, key) = with_two_step(now0).await;
        db.call(move |c| {
            let mut t = now0;
            let mut locks = Vec::new();
            for _ in 0..4 {
                let wrong = wrong_code(&key, t);
                for i in 0..MAX_FAILS {
                    let r = verify(c, &wrong, t, false)?;
                    let want = if i < MAX_FAILS - 1 {
                        Second::Wrong
                    } else {
                        Second::Locked
                    };
                    assert_eq!(r, want, "try {i}");
                }
                // While locked even the right code is refused.
                let right = code_for_step(&key, t / STEP_SECS + 1);
                assert_eq!(verify(c, &right, t, false)?, Second::Locked);
                let lock = lock_remaining_secs(c, t)?;
                locks.push(lock);
                t += lock + 1;
            }
            // Ten minutes, an hour, then six hours at most.
            assert_eq!(locks, vec![600, 3_600, 21_600, 21_600]);
            // A good code starts the ladder again.
            let good = code_for_step(&key, t / STEP_SECS);
            assert_eq!(verify(c, &good, t, false)?, Second::Ok);
            let wrong = wrong_code(&key, t);
            for _ in 0..MAX_FAILS {
                verify(c, &wrong, t, false)?;
            }
            assert_eq!(lock_remaining_secs(c, t)?, 600);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_quiet_day_forgets_earlier_lockouts() {
        let now0 = 1_700_000_000;
        let (db, key) = with_two_step(now0).await;
        db.call(move |c| {
            let wrong = wrong_code(&key, now0);
            for _ in 0..MAX_FAILS {
                verify(c, &wrong, now0, false)?;
            }
            assert_eq!(lock_remaining_secs(c, now0)?, 600);
            // More than a day later, the next lock is ten minutes again.
            let later = now0 + LOCK_FORGET_SECS + 1_000;
            let wrong = wrong_code(&key, later);
            for _ in 0..MAX_FAILS {
                verify(c, &wrong, later, false)?;
            }
            assert_eq!(lock_remaining_secs(c, later)?, 600);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_code_approves_one_protected_action_and_then_waits_for_the_next() {
        let now = 1_700_000_000;
        let (db, key) = with_two_step(now).await;
        db.call(move |c| {
            let cur = now / STEP_SECS;
            let code = code_for_step(&key, cur);
            // The code that signed in may approve one action...
            assert_eq!(verify(c, &code, now, true)?, Second::Ok);
            // ...but not a second, however often it is tried, and that is not
            // counted as a wrong code.
            for _ in 0..10 {
                assert_eq!(verify(c, &code, now, true)?, Second::Spent);
            }
            // The next code works, once.
            let next = code_for_step(&key, cur + 1);
            assert_eq!(verify(c, &next, now + STEP_SECS, true)?, Second::Ok);
            assert_eq!(verify(c, &next, now + STEP_SECS, true)?, Second::Spent);
            // A code that was never valid is still just wrong.
            let wrong = wrong_code(&key, now + STEP_SECS);
            assert_eq!(verify(c, &wrong, now + STEP_SECS, true)?, Second::Wrong);
            // Signing in never reuses a code at all.
            assert_eq!(verify(c, &next, now + STEP_SECS, false)?, Second::Wrong);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn waits_are_said_in_plain_words() {
        assert_eq!(wait_text(0), "a minute");
        assert_eq!(wait_text(30), "a minute");
        assert_eq!(wait_text(61), "2 minutes");
        assert_eq!(wait_text(600), "10 minutes");
        assert_eq!(wait_text(3_600), "60 minutes");
        assert_eq!(wait_text(21_600), "about 6 hours");
    }
}
