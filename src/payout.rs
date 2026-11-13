//! Where the till empties to: the merchant's own wallet address.
//!
//! Set in the setup wizard (no delay: there is no money yet and the wizard
//! is behind the setup code) or later in Settings. Changing it later is the
//! most valuable thing a stolen login could do (redirect future sweeps), so
//! a change needs the admin password AND takes effect after a delay
//! (24 hours by default), is recorded in the security log, shows a warning
//! on Home, and can be cancelled meanwhile.
//!
//! An address set in the server's own configuration
//! (`EBP_SETTLEMENT_DESTINATION`) wins and cannot be changed from the page.

use rusqlite::Connection;

use crate::error::AppError;
use crate::util::now_unix;

const KEY_ACTIVE: &str = "payout_destination";
const KEY_PENDING: &str = "payout_destination_pending";
const KEY_EFFECTIVE_AT: &str = "payout_destination_effective_at";
pub const DEFAULT_DELAY_SECS: i64 = 86_400;

/// The amount the till keeps, when the owner has set one in Settings. Wins over
/// the server's default (`hot_wallet_cap_sats` / `onchain_sweep_cap_sats`).
const KEY_CAP: &str = "sweep_cap_sats";
/// Bounds for that amount. The top is the largest single coin the Ark server
/// accepts (10,000,000 sats): holding more than that on the server defeats the
/// point of sweeping.
pub const MIN_CAP_SATS: i64 = 1;
pub const MAX_CAP_SATS: i64 = 10_000_000;

const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
/// Checksum constants: bech32 (BIP173) and bech32m (BIP350).
const BECH32: u32 = 1;
const BECH32M: u32 = 0x2bc8_30a3;

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk = 1u32;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ u32::from(*v);
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// A bech32 / bech32m string with a good checksum.
struct Decoded {
    hrp: String,
    /// 5-bit values, checksum removed.
    data: Vec<u8>,
    bech32m: bool,
}

/// Decode a lowercase ASCII bech32 string; `None` if the characters or the
/// checksum are wrong.
fn decode_bech32(s: &str) -> Option<Decoded> {
    if !s.is_ascii() {
        return None;
    }
    let pos = s.rfind('1')?;
    let (hrp, rest) = (&s[..pos], &s[pos + 1..]);
    if hrp.is_empty() || rest.len() < 6 {
        return None;
    }
    let mut values: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|c| c & 31));
    let mut data = Vec::with_capacity(rest.len());
    for c in rest.chars() {
        data.push(CHARSET.find(c)? as u8);
    }
    values.extend(&data);
    let bech32m = match polymod(&values) {
        BECH32 => false,
        BECH32M => true,
        _ => return None,
    };
    data.truncate(data.len() - 6);
    Some(Decoded {
        hrp: hrp.to_string(),
        data,
        bech32m,
    })
}

/// Regroup 5-bit values into bytes. Padding must be fewer than 5 bits and
/// all zero (BIP173), otherwise the address is malformed.
fn five_to_eight(data: &[u8]) -> Option<Vec<u8>> {
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut out = Vec::with_capacity(data.len() * 5 / 8);
    for &v in data {
        acc = ((acc << 5) | u32::from(v)) & 0xfff;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || ((acc << (8 - bits)) & 0xff) != 0 {
        return None;
    }
    Some(out)
}

/// A native segwit address must carry a sensible witness program: version
/// 0..=16, 2..=40 bytes, version 0 only 20 or 32 bytes, and bech32 for
/// version 0 but bech32m for every later version (BIP173 / BIP350).
fn segwit_program_ok(d: &Decoded) -> bool {
    let Some((&version, rest)) = d.data.split_first() else {
        return false;
    };
    if version > 16 {
        return false;
    }
    let Some(program) = five_to_eight(rest) else {
        return false;
    };
    if !(2..=40).contains(&program.len()) {
        return false;
    }
    // Only address types that are actually spendable by their owner today:
    // v0 (P2WPKH 20 bytes, P2WSH 32 bytes) and v1 Taproot (32 bytes). Any
    // other witness version or v1 length is valid BIP350 but has no
    // spending rules yet: coins sent there can be taken by anyone. Refused,
    // never paid.
    match version {
        0 => (program.len() == 20 || program.len() == 32) && !d.bech32m,
        1 => program.len() == 32 && d.bech32m,
        _ => false,
    }
}

const NOT_AN_ADDRESS: &str =
    "that doesn't look like an Ark or Bitcoin address; copy it again from your wallet";
const BROKEN_ADDRESS: &str =
    "that address is not valid (a typo, or part of it is missing); copy it again from your wallet";
const SILENT_PAYMENT: &str = "that is a silent payment (BIP-352) address, which this server cannot pay yet: \
    the payment has to be built with the keys of the coins being spent, and the server's wallet does \
    not expose them. Use a regular Bitcoin (bc1\u{2026}) or Ark address from your wallet instead";
const REAL_ADDRESS_ON_TEST_SERVER: &str =
    "this server is in test mode and only works with play-money (signet) coins, \
    but that looks like a real-Bitcoin address. Use a signet address (starts with tb1 or tark1)";
const OLD_TEST_ADDRESS: &str =
    "that is an old-style test-network address, which this server does not use. \
    Use a signet address that starts with tb1 or tark1";
const TEST_ADDRESS_ON_LIVE_SERVER: &str =
    "that is a test-network address, but this server works with real Bitcoin. \
    Use an address that starts with bc1 or ark1";

/// Check a payout address for this server's network (`mode` is `"live"` for
/// real Bitcoin, anything else is the signet/regtest test mode). On success
/// returns the address normalised (trimmed, lowercase); on failure a message
/// the merchant can act on.
///
/// What is checked: the bech32/bech32m checksum (a typo would lose money),
/// for on-chain addresses the witness version and program length, for Ark
/// addresses the bech32m form with the `ark`/`tark` prefix, and that the
/// address belongs to this server's network. Silent payment (BIP-352)
/// addresses are recognised and refused with an explanation.
pub fn check(input: &str, mode: &str) -> Result<String, String> {
    let live = mode == "live";
    let trimmed = input.trim();
    // Older Bitcoin addresses (`1…`, `3…`) are Base58Check, case-sensitive,
    // so they are checked before anything lowercases them.
    if let Some(result) = check_legacy(trimmed, live) {
        return result;
    }
    if trimmed.len() < 14
        || trimmed.len() > 200
        || !trimmed.is_ascii()
        || trimmed.chars().any(char::is_whitespace)
    {
        return Err(NOT_AN_ADDRESS.into());
    }
    // BIP173: all lower case or all upper case, never mixed.
    if trimmed.chars().any(|c| c.is_ascii_lowercase())
        && trimmed.chars().any(|c| c.is_ascii_uppercase())
    {
        return Err(NOT_AN_ADDRESS.into());
    }
    let a = trimmed.to_ascii_lowercase();

    if a.starts_with("sp1") || a.starts_with("tsp1") {
        return Err(SILENT_PAYMENT.into());
    }

    let is_ark = a.starts_with("ark1") || a.starts_with("tark1");
    let is_chain = ["bc1", "tb1", "bcrt1"].iter().any(|p| a.starts_with(p));
    if !is_ark && !is_chain {
        return Err(NOT_AN_ADDRESS.into());
    }
    let test_network = a.starts_with("tark1") || a.starts_with("tb1") || a.starts_with("bcrt1");
    if live && test_network {
        return Err(TEST_ADDRESS_ON_LIVE_SERVER.into());
    }
    if !live && !test_network {
        return Err(REAL_ADDRESS_ON_TEST_SERVER.into());
    }

    let Some(decoded) = decode_bech32(&a) else {
        return Err(BROKEN_ADDRESS.into());
    };
    let good = if is_ark {
        // Ark addresses are bech32m with the `ark` / `tark` prefix.
        decoded.bech32m && (decoded.hrp == "ark" || decoded.hrp == "tark")
    } else {
        matches!(decoded.hrp.as_str(), "bc" | "tb" | "bcrt") && segwit_program_ok(&decoded)
    };
    if good {
        Ok(a)
    } else {
        Err(BROKEN_ADDRESS.into())
    }
}

const BASE58: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// Decode a Base58Check string. Returns the 21-byte payload (version byte
/// plus 20-byte hash) only when the whole thing is exactly 25 bytes and its
/// last 4 bytes match the double-SHA-256 checksum, so a typo is caught.
fn decode_base58check(s: &str) -> Option<Vec<u8>> {
    // Big-endian base-58 to base-256, one character at a time.
    let mut number: Vec<u8> = Vec::new();
    for c in s.bytes() {
        let mut carry = BASE58.iter().position(|&b| b == c)? as u32;
        for byte in number.iter_mut().rev() {
            carry += u32::from(*byte) * 58;
            *byte = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            number.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Each leading '1' stands for one leading zero byte.
    let zeros = s.bytes().take_while(|&c| c == b'1').count();
    let mut raw = vec![0u8; zeros];
    raw.extend(number);
    if raw.len() != 25 {
        return None;
    }
    let (payload, checksum) = raw.split_at(21);
    use sha2::{Digest, Sha256};
    let first = Sha256::digest(payload);
    let second = Sha256::digest(&first[..]);
    if second[..4] != *checksum {
        return None;
    }
    Some(payload.to_vec())
}

/// Legacy (Base58Check) addresses: `1…` (P2PKH, version 0x00) and `3…`
/// (P2SH, 0x05) are real Bitcoin and accepted on a live server; the test
/// forms `m…`/`n…` (0x6f) and `2…` (0xc4) are refused everywhere (test
/// servers use `tb1…`/`tark1…`). `None` means "not a legacy address": the
/// bech32 checks apply.
fn check_legacy(a: &str, live: bool) -> Option<Result<String, String>> {
    let first = a.chars().next()?;
    if !matches!(first, '1' | '3' | 'm' | 'n' | '2')
        || !(26..=35).contains(&a.len())
        || !a.bytes().all(|c| BASE58.contains(&c))
    {
        return None;
    }
    let Some(payload) = decode_base58check(a) else {
        return Some(Err(BROKEN_ADDRESS.into()));
    };
    Some(match payload[0] {
        0x00 | 0x05 if live => Ok(a.to_string()),
        0x00 | 0x05 => Err(REAL_ADDRESS_ON_TEST_SERVER.into()),
        0x6f | 0xc4 if live => Err(TEST_ADDRESS_ON_LIVE_SERVER.into()),
        0x6f | 0xc4 => Err(OLD_TEST_ADDRESS.into()),
        _ => Err(NOT_AN_ADDRESS.into()),
    })
}

/// The 4 bytes at the start of an Ark address (after the version) that name
/// the Ark server it belongs to. Every address the same wallet makes starts
/// with the same 4 bytes (confirmed on five real mainnet addresses from one
/// server); an address from a different server has different ones. `None`
/// when the address does not decode, so the caller says "could not verify"
/// and never guesses.
pub fn ark_server_tag(address: &str) -> Option<[u8; 4]> {
    let d = decode_bech32(&address.trim().to_ascii_lowercase())?;
    if !d.bech32m || !matches!(d.hrp.as_str(), "ark" | "tark") || d.data.len() < 8 {
        return None;
    }
    // The first 5-bit value is the address version; the rest is the payload.
    // Whole bytes only: padding at the end is not needed for the first four.
    let bytes = {
        let (mut acc, mut bits) = (0u32, 0u32);
        let mut out = Vec::new();
        for &v in &d.data[1..] {
            acc = ((acc << 5) | u32::from(v)) & 0xfff;
            bits += 5;
            while bits >= 8 {
                bits -= 8;
                out.push(((acc >> bits) & 0xff) as u8);
            }
        }
        out
    };
    bytes.get(..4)?.try_into().ok()
}

/// What we could find out about a payout address, for the setup page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Passed every check we can run.
    Verified(String),
    /// Looks fine but a check could not run; we do not know.
    Unverified(String),
    /// Failed a check: sending money here could lose it.
    Failed(String),
}

impl Verdict {
    pub fn name(&self) -> &'static str {
        match self {
            Verdict::Verified(_) => "verified",
            Verdict::Unverified(_) => "unverified",
            Verdict::Failed(_) => "failed",
        }
    }
    pub fn message(&self) -> &str {
        match self {
            Verdict::Verified(m) | Verdict::Unverified(m) | Verdict::Failed(m) => m,
        }
    }
}

/// Judge a payout address. `wallet_ark_address` is an Ark address made by
/// this server's own wallet (`None` when it could not be fetched); an Ark
/// payout address must belong to the same Ark server. Nothing here can tell
/// whether the owner controls the address: only that it is well formed and on
/// the right network and server.
pub fn assess(input: &str, mode: &str, wallet_ark_address: Option<&str>) -> Verdict {
    let address = match check(input, mode) {
        Ok(a) => a,
        Err(why) => return Verdict::Failed(why),
    };
    if !(address.starts_with("ark1") || address.starts_with("tark1")) {
        return Verdict::Verified(
            "Verified: a well-formed Bitcoin address for this network. We cannot see whether you control it, so check it is yours."
                .into(),
        );
    }
    let (Some(theirs), Some(reference)) = (
        ark_server_tag(&address),
        wallet_ark_address.and_then(ark_server_tag),
    ) else {
        return Verdict::Unverified(
            "Could not verify: the address is well formed, but we could not compare it with your wallet's Ark server, so we do not know that it works with this server."
                .into(),
        );
    };
    if theirs == reference {
        Verdict::Verified(
            "Verified: a well-formed Ark address on the same Ark server as this wallet. We cannot see whether you control it, so check it is yours."
                .into(),
        )
    } else {
        Verdict::Failed(
            "This Ark address belongs to a different Ark server than this wallet uses. Money sent to it may not arrive, or may be lost."
                .into(),
        )
    }
}

/// [`check`] as a request error, for the setup wizard and the admin page.
pub fn validate(input: &str, mode: &str) -> Result<String, AppError> {
    check(input, mode).map_err(AppError::BadRequest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub active: Option<String>,
    pub pending: Option<String>,
    pub effective_at: Option<i64>,
    /// Set by the server's own configuration; cannot be changed here.
    pub from_server_config: bool,
    /// The amount the till keeps, if the owner set one in Settings.
    pub cap_override: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// Took effect right away (first address, or delay of 0).
    Now,
    /// Will take effect at this time unless cancelled.
    Pending(i64),
}

/// Promote a pending address whose time has come.
fn promote_if_due(conn: &Connection) -> Result<(), AppError> {
    let (Some(pending), Some(at)) = (
        crate::settings::get(conn, KEY_PENDING)?,
        crate::settings::get(conn, KEY_EFFECTIVE_AT)?.and_then(|v| v.parse::<i64>().ok()),
    ) else {
        return Ok(());
    };
    if at > now_unix() {
        return Ok(());
    }
    crate::settings::set(conn, KEY_ACTIVE, &pending)?;
    clear_pending(conn)?;
    audit(
        conn,
        "payout.changed",
        "the payout address change took effect",
    );
    // Phone alert (if on): the owner hears about it even without signing in.
    let _ = crate::alerts::queue_notice(conn, "payout_changed");
    Ok(())
}

fn clear_pending(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM settings WHERE key IN (?1, ?2)",
        rusqlite::params![KEY_PENDING, KEY_EFFECTIVE_AT],
    )?;
    Ok(())
}

fn audit(conn: &Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

fn short(addr: &str) -> String {
    if addr.len() <= 16 {
        return addr.to_string();
    }
    format!("{}…{}", &addr[..8], &addr[addr.len() - 6..])
}

pub fn status(conn: &Connection, server_config: Option<&str>) -> Result<Status, AppError> {
    promote_if_due(conn)?;
    if let Some(cfg) = server_config.filter(|d| !d.trim().is_empty()) {
        return Ok(Status {
            active: Some(cfg.trim().to_string()),
            pending: None,
            effective_at: None,
            from_server_config: true,
            cap_override: cap_override(conn)?,
        });
    }
    Ok(Status {
        active: crate::settings::get(conn, KEY_ACTIVE)?,
        pending: crate::settings::get(conn, KEY_PENDING)?,
        effective_at: crate::settings::get(conn, KEY_EFFECTIVE_AT)?
            .and_then(|v| v.parse::<i64>().ok()),
        from_server_config: false,
        cap_override: cap_override(conn)?,
    })
}

/// The amount the owner chose in Settings, if any. A stored value that is out
/// of range (it cannot be, but a hand-edited database might) is ignored, so
/// the server's default applies instead of something odd.
pub fn cap_override(conn: &Connection) -> Result<Option<i64>, AppError> {
    Ok(crate::settings::get(conn, KEY_CAP)?
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| (MIN_CAP_SATS..=MAX_CAP_SATS).contains(v)))
}

/// How much the till keeps before the rest is sent on to `destination`: the
/// owner's number from Settings, otherwise the server's default for that kind
/// of address (low for Ark, higher for on-chain).
pub fn cap_for(
    conn: &Connection,
    config: &crate::config::Config,
    destination: &str,
) -> Result<i64, AppError> {
    Ok(cap_override(conn)?.unwrap_or_else(|| config.sweep_cap_for(destination)))
}

/// Settings: change the amount the till keeps (the caller has already
/// verified the admin password and, when on, the second step). Takes effect
/// at once: unlike the address it cannot send money anywhere new. Recorded in
/// the security log.
pub fn set_cap(conn: &Connection, cap_sats: i64) -> Result<(), AppError> {
    if !(MIN_CAP_SATS..=MAX_CAP_SATS).contains(&cap_sats) {
        return Err(AppError::BadRequest(format!(
            "the amount must be a whole number of sats between {MIN_CAP_SATS} and {MAX_CAP_SATS}"
        )));
    }
    let before = cap_override(conn)?;
    crate::settings::set(conn, KEY_CAP, &cap_sats.to_string())?;
    audit(
        conn,
        "payout.cap_changed",
        &match before {
            Some(b) => format!("amount kept on the server changed from {b} to {cap_sats} sats"),
            None => format!("amount kept on the server set to {cap_sats} sats"),
        },
    );
    Ok(())
}

/// Settings: go back to the server's default amount.
pub fn clear_cap(conn: &Connection) -> Result<(), AppError> {
    if cap_override(conn)?.is_some() {
        conn.execute("DELETE FROM settings WHERE key = ?1", [KEY_CAP])?;
        audit(
            conn,
            "payout.cap_changed",
            "amount kept on the server reset to the default",
        );
    }
    Ok(())
}

/// The address sweeps should use right now, if any.
pub fn effective(
    conn: &Connection,
    server_config: Option<&str>,
) -> Result<Option<String>, AppError> {
    Ok(status(conn, server_config)?.active)
}

/// Wizard: set the first address immediately.
pub fn set_initial(conn: &Connection, address: &str, mode: &str) -> Result<(), AppError> {
    let a = validate(address, mode)?;
    crate::settings::set(conn, KEY_ACTIVE, &a)?;
    clear_pending(conn)?;
    audit(
        conn,
        "payout.set",
        &format!("payout address set ({})", short(&a)),
    );
    Ok(())
}

/// Wizard, after the owner ticked "I understand money could be lost": save
/// the address even though it failed a check. Sweeps still refuse an address
/// that fails the format checks (see settlement), so this cannot send money
/// anywhere on its own.
pub fn set_initial_unchecked(conn: &Connection, address: &str) -> Result<(), AppError> {
    let a = address.trim().to_string();
    crate::settings::set(conn, KEY_ACTIVE, &a)?;
    clear_pending(conn)?;
    audit(
        conn,
        "payout.set_unverified",
        &format!(
            "payout address set although it failed a check ({}); the owner accepted the risk",
            short(&a)
        ),
    );
    Ok(())
}

/// Settings: change the address (caller has already verified the admin
/// password). The first address applies at once; later ones after `delay`.
pub fn request_change(
    conn: &Connection,
    address: &str,
    delay_secs: i64,
    server_config: Option<&str>,
    mode: &str,
) -> Result<Change, AppError> {
    if server_config.is_some_and(|d| !d.trim().is_empty()) {
        return Err(AppError::BadRequest(
            "the payout address is set in the server's configuration; change it there".into(),
        ));
    }
    let a = validate(address, mode)?;
    promote_if_due(conn)?;
    let current = crate::settings::get(conn, KEY_ACTIVE)?;
    if current.as_deref() == Some(a.as_str()) {
        clear_pending(conn)?;
        return Ok(Change::Now);
    }
    if current.is_none() || delay_secs <= 0 {
        crate::settings::set(conn, KEY_ACTIVE, &a)?;
        clear_pending(conn)?;
        if current.is_some() {
            let _ = crate::alerts::queue_notice(conn, "payout_changed");
        }
        audit(
            conn,
            "payout.set",
            &format!("payout address set ({})", short(&a)),
        );
        return Ok(Change::Now);
    }
    let at = now_unix() + delay_secs;
    crate::settings::set(conn, KEY_PENDING, &a)?;
    crate::settings::set(conn, KEY_EFFECTIVE_AT, &at.to_string())?;
    audit(
        conn,
        "payout.change_requested",
        &format!(
            "payout address change requested ({}), takes effect after the safety delay",
            short(&a)
        ),
    );
    // The safety delay only helps if the owner hears about the change.
    let _ = crate::alerts::queue_notice(conn, "payout_requested");
    Ok(Change::Pending(at))
}

pub fn cancel(conn: &Connection) -> Result<bool, AppError> {
    let had = crate::settings::get(conn, KEY_PENDING)?.is_some();
    clear_pending(conn)?;
    if had {
        audit(
            conn,
            "payout.change_cancelled",
            "pending payout address change cancelled",
        );
    }
    Ok(had)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn the_amount_kept_can_be_set_changed_and_reset() {
        let c = conn();
        let cfg = crate::config::Config::default();
        // Nothing saved: the server's default for each kind of address.
        assert_eq!(cap_override(&c).unwrap(), None);
        assert_eq!(
            cap_for(&c, &cfg, "ark1abc").unwrap(),
            cfg.hot_wallet_cap_sats
        );
        assert_eq!(
            cap_for(&c, &cfg, "bc1qabc").unwrap(),
            cfg.onchain_sweep_cap_sats
        );
        // The owner's number wins for both kinds.
        set_cap(&c, 5_000).unwrap();
        assert_eq!(cap_override(&c).unwrap(), Some(5_000));
        assert_eq!(cap_for(&c, &cfg, "ark1abc").unwrap(), 5_000);
        assert_eq!(cap_for(&c, &cfg, "bc1qabc").unwrap(), 5_000);
        set_cap(&c, 2_500).unwrap();
        assert_eq!(cap_for(&c, &cfg, "ark1abc").unwrap(), 2_500);
        // Reset goes back to the defaults.
        clear_cap(&c).unwrap();
        assert_eq!(cap_override(&c).unwrap(), None);
        assert_eq!(
            cap_for(&c, &cfg, "ark1abc").unwrap(),
            cfg.hot_wallet_cap_sats
        );
    }

    #[test]
    fn the_amount_kept_must_be_in_range_and_bad_stored_values_are_ignored() {
        let c = conn();
        for bad in [-5, 0, MAX_CAP_SATS + 1, i64::MAX] {
            assert!(set_cap(&c, bad).is_err(), "{bad} must be refused");
        }
        assert_eq!(cap_override(&c).unwrap(), None);
        set_cap(&c, MIN_CAP_SATS).unwrap();
        set_cap(&c, MAX_CAP_SATS).unwrap();
        // A hand-edited database cannot make the cap zero, negative or text.
        for junk in ["0", "-1", "abc", "", "99999999999"] {
            crate::settings::set(&c, KEY_CAP, junk).unwrap();
            assert_eq!(cap_override(&c).unwrap(), None, "{junk:?} must be ignored");
        }
    }

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        c
    }

    // Real Ark-style addresses (bech32m, prefixes `ark` / `tark`), made for
    // these tests; the payload is not meaningful, the checksum is.
    pub(crate) const TARK_A: &str =
        "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
    pub(crate) const ARK_A: &str =
        "ark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqqnaa07n";

    #[test]
    fn onchain_addresses_follow_bip173_and_bip350() {
        // Test mode (signet / regtest): testnet-family addresses.
        for ok in [
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
            "tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c",
            "  TB1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KXPJZSX ",
        ] {
            assert!(check(ok, "test").is_ok(), "{ok}");
        }
        // Live mode: mainnet v0 (20/32 bytes) and v1 Taproot (32 bytes).
        for ok in [
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            "BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4",
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0",
        ] {
            assert!(check(ok, "live").is_ok(), "{ok}");
        }
        // Valid BIP350 but anyone-can-spend today (future witness versions,
        // v1 with a non-32-byte program): money sent there would be lost.
        for lost in [
            "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y",
            "bc1zw508d6qejxtdg4y5r3zarvaryvaxxpcs",
        ] {
            assert!(check(lost, "live").is_err(), "{lost}");
        }
        // Official BIP350 invalid vectors and common mistakes (all refused
        // on both networks).
        for bad in [
            "tc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq5zuyut", // wrong prefix
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqh2y7hd", // v1 with bech32
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh",                     // v0 with bech32m
            "tb1z0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqglt7rf", // v2 with bech32
            "bc1p38j9r5y49hruaue7wxjce0updqjuyyx0kh56v8s25huc6995vvpql3jow4", // invalid character
            "bc1pw5dgrnzv",                                                   // program too short
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7v07qwwzcrf", // program too long
            "BC1QR508D6QEJXTDG4Y5R3ZARVARYV98GJ9P",                           // v0 with 16 bytes
            "tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq47Zagq", // mixed case
            "tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vpggkg4j", // padding > 4 bits
            "bc1gmk9yu",                                                      // empty data
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsy",                     // typo
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3",                       // truncated
            "hello world, not an address",
            "",
        ] {
            assert!(check(bad, "test").is_err(), "test: {bad}");
            assert!(check(bad, "live").is_err(), "live: {bad}");
        }
    }

    #[test]
    fn legacy_mainnet_addresses_are_accepted_on_a_live_server_only() {
        // Known-good vectors: P2PKH (1…) and P2SH (3…). Case is kept.
        for ok in [
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
        ] {
            assert_eq!(check(ok, "live").unwrap(), ok, "{ok}");
            assert_eq!(check(&format!("  {ok} "), "live").unwrap(), ok);
            let e = check(ok, "test").unwrap_err();
            assert!(e.contains("test mode"), "{e}");
        }
        // One changed character (or a changed case) breaks the checksum.
        for typo in [
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3",
            "1bvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLz",
        ] {
            let e = check(typo, "live").unwrap_err();
            assert!(e.contains("not valid"), "{typo}: {e}");
        }
        // Too short / too long / characters base58 never uses.
        assert!(check("1BvBMSEYstWetqTFn5Au4m4GFg7xJaN", "live").is_err());
        assert!(check("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2VN2", "live").is_err());
        assert!(check("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN0", "live").is_err());
        // Test-network legacy forms (m…, n…, 2…) are refused in both modes.
        for t in [
            "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn",
            "n3GNqMveyvaPvUbH469vDRadqpJMPc84JA",
            "2MzQwSSnBHWHqSAqtTVQ6v47XtaisrJa1Vc",
        ] {
            assert!(check(t, "live").is_err(), "live: {t}");
            assert!(check(t, "test").is_err(), "test: {t}");
        }
    }

    #[test]
    fn base58check_decodes_a_known_address() {
        let p = decode_base58check("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").unwrap();
        assert_eq!(p[0], 0x00);
        assert_eq!(
            hex::encode(&p[1..]),
            "62e907b15cbf27d5425399ebf6f0fb50ebb88f18"
        );
        let p = decode_base58check("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy").unwrap();
        assert_eq!(p[0], 0x05);
        assert!(decode_base58check("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb").is_none());
    }

    #[test]
    fn addresses_must_belong_to_this_servers_network() {
        // A real-money address on a test server: the dangerous direction.
        let e = check("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "test").unwrap_err();
        assert!(e.contains("test mode"), "{e}");
        assert!(check(ARK_A, "test").is_err());
        // A test address on a live server.
        let e = check("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx", "live").unwrap_err();
        assert!(e.contains("test-network"), "{e}");
        assert!(check(TARK_A, "live").is_err());
        // Regtest is a test network.
        assert!(
            check("bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080", "test").is_ok(),
            "regtest address in test mode"
        );
    }

    #[test]
    fn ark_addresses_need_bech32m_with_the_right_prefix() {
        assert_eq!(check(TARK_A, "test").unwrap(), TARK_A);
        assert_eq!(check(ARK_A, "live").unwrap(), ARK_A);
        // One changed character breaks the checksum.
        let typo = TARK_A.replacen("pe2t", "pe2u", 1);
        assert!(check(&typo, "test").is_err());
        assert!(check("tark1BIO!!!!!!!!!!!!!", "test").is_err());
        assert!(check("tark1", "test").is_err());
        // A plain bech32 (not bech32m) string with an ark prefix is refused.
        assert!(check("tark1qw508d6qejxtdg4y5r3zarvary0c5xw7k", "test").is_err());
    }

    // Two addresses made by throwaway mainnet test wallets on Second's server.
    const SAME_SERVER_A: &str = "ark1pu6h30w3zqqp925mq7na5tp0g9e38dr0ehcm5knhn3t54nyn5vv96stcu993fkxpzqyp68xugqvak7kyzzpjeuwg8sa0083yvkts40sl0vah9wa9z9sl8ulcdexg9g";
    const SAME_SERVER_B: &str = "ark1pu6h30w3zqqpsgvg9tfp333jaffnfye2q0lase00lkl4wg4h2c5x5hytk0jeycn3zqyp9kccnvcewvcznplaqzmurtq6sggy8zmxerejhdcpwcukycpfdf6sy0sp9h";

    #[test]
    fn addresses_of_one_ark_server_share_a_tag_and_others_do_not() {
        assert_eq!(ark_server_tag(SAME_SERVER_A), ark_server_tag(SAME_SERVER_B));
        assert!(ark_server_tag(SAME_SERVER_A).is_some());
        assert_ne!(ark_server_tag(SAME_SERVER_A), ark_server_tag(ARK_A));
        assert_eq!(ark_server_tag("tark1"), None);
        assert_eq!(ark_server_tag("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"), None);
    }

    #[test]
    fn the_verdict_says_verified_unverified_or_failed() {
        // Same server: verified.
        let v = assess(SAME_SERVER_A, "live", Some(SAME_SERVER_B));
        assert_eq!(v.name(), "verified");
        // Wallet address unknown: could not verify, still allowed.
        let v = assess(SAME_SERVER_A, "live", None);
        assert_eq!(v.name(), "unverified");
        assert!(v.message().starts_with("Could not verify"));
        // A reference that does not decode: could not verify, never "verified".
        assert_eq!(assess(SAME_SERVER_A, "live", Some("tark1qabc")).name(), "unverified");
        // A different Ark server: failed.
        assert_eq!(assess(ARK_A, "live", Some(SAME_SERVER_A)).name(), "failed");
        // One character changed: the checksum fails.
        let mut typo = SAME_SERVER_A.to_string();
        let swap = if &typo[30..31] == "q" { "p" } else { "q" };
        typo.replace_range(30..31, swap);
        assert_eq!(assess(&typo, "live", Some(SAME_SERVER_B)).name(), "failed");
        // A play-money address on a real-money server: failed.
        assert_eq!(assess(TARK_A, "live", Some(SAME_SERVER_B)).name(), "failed");
        // A good Bitcoin address: verified as well formed.
        assert_eq!(
            assess("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "live", None).name(),
            "verified"
        );
    }

    #[test]
    fn an_address_that_failed_can_be_saved_only_on_purpose() {
        let c = conn();
        assert!(set_initial(&c, "tark1notanaddress", "test").is_err());
        set_initial_unchecked(&c, " tark1notanaddress ").unwrap();
        assert_eq!(
            crate::settings::get(&c, KEY_ACTIVE).unwrap().as_deref(),
            Some("tark1notanaddress")
        );
    }

    #[test]
    fn silent_payment_addresses_get_a_clear_explanation() {
        for sp in [
            "sp1qhcvtshmhlspymvme4nceazsuuc3s02mmkx7289fcnm8u9kh67aqmux9ctamlcqjdkdu6euv73gwwvgc84damr09rj5ufan7zmta0wsgpuu3a7e",
            "tsp1qwwa0swy9t629kgs4zu3jvmvwyg8j93mvgvc573yf6esd9wrqt2th8whc8zz4a9zmyg23wgexdk8zyrezcakyxv20gjyavcxjhps949cpshyxtg",
        ] {
            for mode in ["test", "live"] {
                let e = check(sp, mode).unwrap_err();
                assert!(e.contains("silent payment"), "{e}");
            }
        }
    }

    #[test]
    fn first_address_is_immediate_later_changes_wait_and_can_be_cancelled() {
        let c = conn();
        let a = TARK_A;
        let b = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        assert_eq!(
            request_change(&c, a, 3600, None, "test").unwrap(),
            Change::Now
        );
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(a));

        let Change::Pending(at) = request_change(&c, b, 3600, None, "test").unwrap() else {
            panic!("second change must wait")
        };
        assert!(at > now_unix());
        // Still paying out to the OLD address until the delay passes.
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(a));
        let s = status(&c, None).unwrap();
        assert_eq!(s.pending.as_deref(), Some(b));

        assert!(cancel(&c).unwrap());
        assert_eq!(status(&c, None).unwrap().pending, None);
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(a));
    }

    #[test]
    fn a_due_change_takes_effect() {
        let c = conn();
        let a = TARK_A;
        let b = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        request_change(&c, a, 3600, None, "test").unwrap();
        request_change(&c, b, 3600, None, "test").unwrap();
        // Pretend the delay has passed.
        let queued = || {
            crate::settings::get(&c, "alert_notices")
                .unwrap()
                .unwrap_or_default()
        };
        assert_eq!(
            queued(),
            "payout_requested",
            "the owner's phone hears about the request"
        );
        crate::settings::set(&c, KEY_EFFECTIVE_AT, &(now_unix() - 1).to_string()).unwrap();
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(b));
        assert_eq!(status(&c, None).unwrap().pending, None);
        assert_eq!(
            queued(),
            "payout_requested,payout_changed",
            "and about it taking effect"
        );
    }

    #[test]
    fn server_config_wins_and_blocks_changes() {
        let c = conn();
        let cfg = Some("tark1fromconfigfromconfigfromconfig");
        assert_eq!(effective(&c, cfg).unwrap().as_deref(), cfg);
        assert!(status(&c, cfg).unwrap().from_server_config);
        assert!(request_change(&c, TARK_A, 0, cfg, "test").is_err());
    }
}
