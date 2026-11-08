//! Limits on what a shop's API key can do, and the "pause new payments" switch.
//!
//! Why: the shop's key lives inside the shop's website. If that website is
//! ever taken over (for example through a bug in PHP or WordPress), the key is
//! what an attacker gets. So (1) the key can only do what a shop needs, (2) the
//! owner can lock it to the shop's own address(es), and (3) the owner can stop
//! new payment requests with one switch, which the shop cannot undo.
//!
//! State lives in the `settings` table:
//! * `payments_paused`  — `true` while new payment requests are refused
//! * `api_allowed_ips`  — optional comma-separated list of addresses the key
//!   may be used from (empty = anywhere)
//! * `api_last_blocked` — `address|unix time` of the latest refused request

use std::net::IpAddr;

use rusqlite::Connection;

use crate::error::AppError;
use crate::settings;
use crate::util::now_unix;

const PAUSED: &str = "payments_paused";
const IPS: &str = "api_allowed_ips";
const BLOCKED: &str = "api_last_blocked";
const MAX_IPS: usize = 10;

pub fn is_paused(conn: &Connection) -> Result<bool, AppError> {
    settings::get_bool(conn, PAUSED)
}

pub fn set_paused(conn: &Connection, paused: bool) -> Result<(), AppError> {
    settings::set(conn, PAUSED, if paused { "true" } else { "false" })
}

/// The addresses the key is limited to (empty = no limit).
pub fn allowed(conn: &Connection) -> Result<Vec<String>, AppError> {
    Ok(settings::get(conn, IPS)?
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Is a request from `ip` allowed to use the shop key?
pub fn ip_allowed(conn: &Connection, ip: &str) -> Result<bool, AppError> {
    let list = allowed(conn)?;
    if list.is_empty() {
        return Ok(true);
    }
    let Ok(ip) = ip.parse::<IpAddr>() else {
        return Ok(false);
    };
    Ok(list
        .iter()
        .filter_map(|a| a.parse::<IpAddr>().ok())
        .any(|a| a == ip))
}

/// Save the limit. Accepts addresses separated by commas, spaces or new lines;
/// an empty value removes the limit.
pub fn set_allowed(conn: &Connection, input: &str) -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = Vec::new();
    for part in input.split(|c: char| c == ',' || c.is_whitespace()) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let ip: IpAddr = part.parse().map_err(|_| {
            AppError::BadRequest(format!(
                "\"{part}\" is not an address. Use numbers like 203.0.113.7, separated by commas."
            ))
        })?;
        let ip = ip.to_string();
        if !out.contains(&ip) {
            out.push(ip);
        }
    }
    if out.len() > MAX_IPS {
        return Err(AppError::BadRequest(format!(
            "At most {MAX_IPS} addresses, please."
        )));
    }
    settings::set(conn, IPS, &out.join(","))?;
    if out.is_empty() {
        settings::set(conn, BLOCKED, "")?;
    }
    Ok(out)
}

/// Remember that a request was refused because of the limit (at most one
/// write a minute per address, so a flood cannot hammer the database).
pub fn note_blocked(conn: &Connection, ip: &str) -> Result<(), AppError> {
    let now = now_unix();
    if let Some((last_ip, at)) = last_blocked(conn)? {
        if last_ip == ip && now - at < 60 {
            return Ok(());
        }
    }
    settings::set(conn, BLOCKED, &format!("{ip}|{now}"))
}

pub fn last_blocked(conn: &Connection) -> Result<Option<(String, i64)>, AppError> {
    Ok(settings::get(conn, BLOCKED)?.and_then(|v| {
        let (ip, at) = v.split_once('|')?;
        Some((ip.to_string(), at.parse().ok()?))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        // The settings helpers create their table on first use.
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn no_limit_means_anywhere() {
        let c = conn();
        assert!(ip_allowed(&c, "198.51.100.9").unwrap());
        assert!(ip_allowed(&c, "unknown").unwrap());
    }

    #[test]
    fn a_limit_allows_only_listed_addresses() {
        let c = conn();
        let saved = set_allowed(&c, "203.0.113.7, 2001:db8::1\n203.0.113.7").unwrap();
        assert_eq!(saved, vec!["203.0.113.7", "2001:db8::1"]);
        assert!(ip_allowed(&c, "203.0.113.7").unwrap());
        assert!(ip_allowed(&c, "2001:0db8:0:0:0:0:0:1").unwrap());
        assert!(!ip_allowed(&c, "203.0.113.8").unwrap());
        assert!(!ip_allowed(&c, "unknown").unwrap());
        // Removing the limit.
        set_allowed(&c, "  ").unwrap();
        assert!(ip_allowed(&c, "203.0.113.8").unwrap());
    }

    #[test]
    fn bad_input_is_refused_in_plain_words() {
        let c = conn();
        let e = set_allowed(&c, "my-shop.com").unwrap_err().to_string();
        assert!(
            e.contains("my-shop.com") && e.contains("not an address"),
            "{e}"
        );
        assert!(set_allowed(
            &c,
            &(1..=11)
                .map(|i| format!("10.0.0.{i}"))
                .collect::<Vec<_>>()
                .join(",")
        )
        .is_err());
    }

    #[test]
    fn blocked_requests_are_remembered_but_not_written_every_time() {
        let c = conn();
        note_blocked(&c, "198.51.100.9").unwrap();
        let (ip, at) = last_blocked(&c).unwrap().unwrap();
        assert_eq!(ip, "198.51.100.9");
        // Same address again within a minute: the stored time is unchanged.
        settings::set(&c, BLOCKED, &format!("198.51.100.9|{}", at - 10)).unwrap();
        note_blocked(&c, "198.51.100.9").unwrap();
        assert_eq!(last_blocked(&c).unwrap().unwrap().1, at - 10);
    }

    #[test]
    fn pause_switch_round_trips() {
        let c = conn();
        assert!(!is_paused(&c).unwrap());
        set_paused(&c, true).unwrap();
        assert!(is_paused(&c).unwrap());
        set_paused(&c, false).unwrap();
        assert!(!is_paused(&c).unwrap());
    }
}
