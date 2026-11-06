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

const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

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

/// Bech32 or bech32m checksum of a lowercase address (BIP173 / BIP350).
fn bech32_checksum_ok(s: &str) -> bool {
    let Some(pos) = s.rfind('1') else {
        return false;
    };
    let (hrp, data) = (&s[..pos], &s[pos + 1..]);
    if hrp.is_empty() || data.len() < 6 {
        return false;
    }
    let mut values: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|c| c & 31));
    for c in data.chars() {
        match CHARSET.find(c) {
            Some(i) => values.push(i as u8),
            None => return false,
        }
    }
    let c = polymod(&values);
    c == 1 || c == 0x2bc8_30a3
}

fn bech32_charset_only(rest: &str) -> bool {
    !rest.is_empty() && rest.chars().all(|c| CHARSET.contains(c))
}

/// Check a payout address and return it normalised (trimmed, lowercase).
/// On-chain addresses get a full checksum check (a typo would lose money).
/// Ark addresses are checked for shape only.
pub fn validate(input: &str) -> Result<String, AppError> {
    let bad = || {
        AppError::BadRequest(
            "that doesn't look like an Ark or Bitcoin address; copy it again from your wallet"
                .into(),
        )
    };
    let a = input.trim().to_ascii_lowercase();
    if a.len() < 14 || a.len() > 200 || !a.is_ascii() || a.chars().any(char::is_whitespace) {
        return Err(bad());
    }
    if let Some(rest) = a.strip_prefix("tark1").or_else(|| a.strip_prefix("ark1")) {
        return bech32_charset_only(rest).then_some(a).ok_or_else(bad);
    }
    if ["bc1", "tb1", "bcrt1"].iter().any(|p| a.starts_with(p)) {
        return bech32_checksum_ok(&a).then_some(a).ok_or_else(|| {
            AppError::BadRequest(
                "that address has a typo in it (its check characters don't match); copy it again"
                    .into(),
            )
        });
    }
    Err(bad())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub active: Option<String>,
    pub pending: Option<String>,
    pub effective_at: Option<i64>,
    /// Set by the server's own configuration; cannot be changed here.
    pub from_server_config: bool,
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
        });
    }
    Ok(Status {
        active: crate::settings::get(conn, KEY_ACTIVE)?,
        pending: crate::settings::get(conn, KEY_PENDING)?,
        effective_at: crate::settings::get(conn, KEY_EFFECTIVE_AT)?
            .and_then(|v| v.parse::<i64>().ok()),
        from_server_config: false,
    })
}

/// The address sweeps should use right now, if any.
pub fn effective(
    conn: &Connection,
    server_config: Option<&str>,
) -> Result<Option<String>, AppError> {
    Ok(status(conn, server_config)?.active)
}

/// Wizard: set the first address immediately.
pub fn set_initial(conn: &Connection, address: &str) -> Result<(), AppError> {
    let a = validate(address)?;
    crate::settings::set(conn, KEY_ACTIVE, &a)?;
    clear_pending(conn)?;
    audit(
        conn,
        "payout.set",
        &format!("payout address set ({})", short(&a)),
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
) -> Result<Change, AppError> {
    if server_config.is_some_and(|d| !d.trim().is_empty()) {
        return Err(AppError::BadRequest(
            "the payout address is set in the server's configuration; change it there".into(),
        ));
    }
    let a = validate(address)?;
    promote_if_due(conn)?;
    let current = crate::settings::get(conn, KEY_ACTIVE)?;
    if current.as_deref() == Some(a.as_str()) {
        clear_pending(conn)?;
        return Ok(Change::Now);
    }
    if current.is_none() || delay_secs <= 0 {
        crate::settings::set(conn, KEY_ACTIVE, &a)?;
        clear_pending(conn)?;
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
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        c
    }

    #[test]
    fn onchain_addresses_need_a_valid_checksum() {
        // BIP173 / BIP350 test vectors.
        for ok in [
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0",
            "  BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4 ",
        ] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for bad in [
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5", // typo
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3",   // truncated
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",         // legacy: not supported
            "hello world, not an address",
            "",
        ] {
            assert!(validate(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ark_addresses_are_checked_for_shape() {
        assert!(validate("tark1qw508d6qejxtdg4y5r3zarvary0c5xw7k").is_ok());
        assert!(validate("ark1qw508d6qejxtdg4y5r3zarvary0c5xw7k").is_ok());
        assert!(validate("tark1BIO!!!!!!!!!!!!!").is_err());
        assert!(validate("tark1").is_err());
    }

    #[test]
    fn first_address_is_immediate_later_changes_wait_and_can_be_cancelled() {
        let c = conn();
        let a = "tark1qw508d6qejxtdg4y5r3zarvary0c5xw7k";
        let b = "tark1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
        assert_eq!(request_change(&c, a, 3600, None).unwrap(), Change::Now);
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(a));

        let Change::Pending(at) = request_change(&c, b, 3600, None).unwrap() else {
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
        let a = "tark1qw508d6qejxtdg4y5r3zarvary0c5xw7k";
        let b = "tark1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
        request_change(&c, a, 3600, None).unwrap();
        request_change(&c, b, 3600, None).unwrap();
        // Pretend the delay has passed.
        crate::settings::set(&c, KEY_EFFECTIVE_AT, &(now_unix() - 1).to_string()).unwrap();
        assert_eq!(effective(&c, None).unwrap().as_deref(), Some(b));
        assert_eq!(status(&c, None).unwrap().pending, None);
    }

    #[test]
    fn server_config_wins_and_blocks_changes() {
        let c = conn();
        let cfg = Some("tark1fromconfigfromconfigfromconfig");
        assert_eq!(effective(&c, cfg).unwrap().as_deref(), cfg);
        assert!(status(&c, cfg).unwrap().from_server_config);
        assert!(request_change(&c, "tark1qw508d6qejxtdg4y5r3zarvary0c5xw7k", 0, cfg).is_err());
    }
}
