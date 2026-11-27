//! Moving from the free sslip.io address to the owner's own domain
//! (`docs/OWN_DOMAIN.md`). In testing, behind a Settings switch.
//!
//! The move never swaps one name for another in a single step:
//! 1. **check**: the new name must already reach this very server (a one-time
//!    token fetched over plain http through the new name);
//! 2. **serve**: the web server serves the new name next to the current one and
//!    gets its certificate; checked over https with the token again;
//! 3. **make main**: new links use the new name; the old one stays as a backup
//!    (passkeys made there keep working there);
//! 4. **watch**: for 14 days the new name is checked several times a day;
//! 5. **retire**: 30 days after step 3, if every check passed and the shop is
//!    calling the new name, the sslip.io backup is given up (warned 3 days
//!    ahead; the owner can postpone). A failed check keeps the backup.
//!
//! "Go back" makes the backup the main address again at any time.
//!
//! Changes to the web server are made by the address helper
//! (`scripts/address-helper.sh`, root, no network port), which takes a strict
//! request from `/updates/inbox/address-request.json`. Every name is checked
//! here and again by the helper. All decisions are made here, in Rust.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::settings;
use crate::util::now_unix;

/// Settings key: the Settings switch (off by default while in testing).
pub const ENABLED_KEY: &str = "own_domain_enabled";
/// Settings key: the move's state (JSON, [`MoveState`]).
pub const STATE_KEY: &str = "own_domain_state";
/// Settings key: the host name the shop last called with its key.
pub const SHOP_HOST_KEY: &str = "own_domain_shop_host";

/// Days of checks before the move counts as working.
pub const WATCH_DAYS: i64 = 14;
/// Days after the switch when the sslip.io backup is given up (if all is well).
pub const RETIRE_DAYS: i64 = 30;
/// How many days ahead the owner is warned before the backup is given up.
pub const RETIRE_WARN_DAYS: i64 = 3;
/// How long "keep it a little longer" postpones giving up the backup.
pub const POSTPONE_DAYS: i64 = 7;
/// How long a check token is valid.
pub const TOKEN_SECS: i64 = 30 * 60;
/// Seconds between watch checks.
pub const WATCH_EVERY_SECS: i64 = 6 * 3600;
const DAY: i64 = 86_400;

pub const NOT_A_NAME: &str =
    "That is not a domain name like pay.myshop.com. Use small letters, digits, dots and dashes.";

/// Where a move stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Nothing started (or a move was abandoned).
    #[default]
    Idle,
    /// The new name reaches this server; nothing changed yet.
    Checked,
    /// Asked the helper to serve the new name next to the current one.
    ServeRequested,
    /// Both names are served; the new one has a working certificate.
    Serving,
    /// Asked the helper to make the new name the main address.
    MainRequested,
    /// The new name is the main address; the backup still works. Watching.
    Main,
    /// Asked the helper to give up the backup.
    RetireRequested,
    /// Done: only the new name is served.
    Retired,
    /// Asked the helper to make the backup the main address again.
    BackRequested,
}

/// The move, saved under [`STATE_KEY`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveState {
    pub step: Step,
    /// The owner's own name being moved to.
    pub name: Option<String>,
    /// The address that was the main one before the move (the backup).
    pub backup: Option<String>,
    /// One-time token for the checks (the `/.well-known/ebp-check/` page).
    pub token: Option<String>,
    pub token_expires: i64,
    /// When the new name became the main address.
    pub switched_at: Option<i64>,
    pub last_check_at: Option<i64>,
    pub last_ok_at: Option<i64>,
    /// Failed watch checks since the switch (any failure keeps the backup).
    pub failures: u32,
    /// Plain-words reason of the last failed check.
    pub last_problem: Option<String>,
    /// The owner asked to keep the backup until this time.
    pub keep_backup_until: Option<i64>,
    /// The helper request id we are waiting for.
    pub request_id: Option<String>,
    /// Phone notices already sent for this move (so each goes once).
    #[serde(default)]
    pub told: Vec<String>,
}

pub fn is_enabled(conn: &Connection) -> Result<bool, AppError> {
    Ok(settings::get(conn, ENABLED_KEY)?.as_deref() == Some("1"))
}

pub fn load(conn: &Connection) -> Result<MoveState, AppError> {
    Ok(settings::get(conn, STATE_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

pub fn save(conn: &Connection, st: &MoveState) -> Result<(), AppError> {
    let json = serde_json::to_string(st).map_err(|e| AppError::Internal(e.to_string()))?;
    settings::set(conn, STATE_KEY, &json)
}

/// A plain host name the owner may move to: lower case, dots between labels,
/// no IP address, at most 253 characters, not a free sslip.io-style name.
/// Returns the cleaned name (trimmed, lower case, no trailing dot or scheme).
pub fn clean_name(raw: &str) -> Result<String, AppError> {
    let mut s = raw.trim().to_ascii_lowercase();
    for p in ["https://", "http://"] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.to_string();
        }
    }
    let s = s.trim_end_matches('/').trim_end_matches('.').to_string();
    let bad = || AppError::BadRequest(NOT_A_NAME.into());
    if s.is_empty() || s.len() > 253 || s.parse::<IpAddr>().is_ok() {
        return Err(bad());
    }
    let labels: Vec<&str> = s.split('.').collect();
    if labels.len() < 2 {
        return Err(bad());
    }
    for l in &labels {
        let ok = !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(bad());
        }
    }
    let tld = labels[labels.len() - 1];
    if !tld.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(bad());
    }
    if is_free_ip_name(&s) {
        return Err(AppError::BadRequest(
            "That is a free address made from an IP. Use a domain you own, like pay.myshop.com."
                .into(),
        ));
    }
    Ok(s)
}

/// sslip.io / nip.io style names, which are made from an IP address.
pub fn is_free_ip_name(host: &str) -> bool {
    ["sslip.io", "nip.io"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// The IPv4 address an sslip.io name stands for (`203-0-113-7.sslip.io`).
pub fn sslip_ip(host: &str) -> Option<IpAddr> {
    let first = host.strip_suffix(".sslip.io")?;
    let first = first.rsplit('.').next()?;
    first
        .replace('-', ".")
        .parse::<std::net::Ipv4Addr>()
        .ok()
        .map(IpAddr::V4)
}

/// The host part of a full address (`https://pay.example.com/x` → `pay.example.com`).
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Cloudflare's published proxy address ranges (cloudflare.com/ips). A name
/// that resolves into one of them has the orange cloud (proxy) turned on.
const CLOUDFLARE_V4: &[(u32, u8)] = &[
    (0xAD_F5_30_00, 20), // 173.245.48.0/20
    (0x67_15_F4_00, 22), // 103.21.244.0/22
    (0x67_16_C8_00, 22), // 103.22.200.0/22
    (0x67_1F_04_00, 22), // 103.31.4.0/22
    (0x8D_65_40_00, 18), // 141.101.64.0/18
    (0x6C_A2_C0_00, 18), // 108.162.192.0/18
    (0xBE_5D_F0_00, 20), // 190.93.240.0/20
    (0xBC_72_60_00, 20), // 188.114.96.0/20
    (0xC5_EA_F0_00, 22), // 197.234.240.0/22
    (0xC6_29_80_00, 17), // 198.41.128.0/17
    (0xA2_9E_00_00, 15), // 162.158.0.0/15
    (0x68_10_00_00, 13), // 104.16.0.0/13
    (0x68_18_00_00, 14), // 104.24.0.0/14
    (0xAC_40_00_00, 13), // 172.64.0.0/13
    (0x83_00_48_00, 22), // 131.0.72.0/22
];
const CLOUDFLARE_V6: &[(u128, u8)] = &[
    (0x2400_cb00 << 96, 32),
    (0x2606_4700 << 96, 32),
    (0x2803_f800 << 96, 32),
    (0x2405_b500 << 96, 32),
    (0x2405_8100 << 96, 32),
    (0x2a06_98c0 << 96, 29),
    (0x2c0f_f248 << 96, 32),
];

pub fn is_cloudflare_proxy(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let n = u32::from(v4);
            CLOUDFLARE_V4
                .iter()
                .any(|(net, len)| n >> (32 - len) == net >> (32 - len))
        }
        IpAddr::V6(v6) => {
            let n = u128::from(v6);
            CLOUDFLARE_V6
                .iter()
                .any(|(net, len)| n >> (128 - len) == net >> (128 - len))
        }
    }
}

/// What the check found, in plain words for the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Finding {
    /// The name reaches this server: ready for the next step.
    Ready,
    /// The name does not exist yet (or the record is not visible yet).
    NotThereYet,
    /// The name goes through Cloudflare's proxy (orange cloud).
    CloudflareProxy,
    /// The name points at other addresses.
    Elsewhere { found: Vec<String> },
    /// The IPv4 record is right, but an IPv6 record points elsewhere.
    Ipv6Elsewhere { found: Vec<String> },
    /// The name points here but the check page did not answer (ports, firewall).
    NoAnswer,
    /// https on the new name does not work yet (certificate not ready or refused).
    NoCertificate,
}

impl Finding {
    pub fn words(&self) -> String {
        match self {
            Finding::Ready => "Ready: this name reaches your server.".into(),
            Finding::NotThereYet => "This name is not there yet. New records can take a few minutes (a new Cloudflare domain up to 24 hours). Check again later.".into(),
            Finding::CloudflareProxy => "This name goes through Cloudflare's proxy. In Cloudflare, open the record and turn the orange cloud off (Proxy status: DNS only), then check again.".into(),
            Finding::Elsewhere { found } => format!("This name points at {}, not at this server. Change the A record's address to this server's IP, then check again.", found.join(", ")),
            Finding::Ipv6Elsewhere { found } => format!("The A record is right, but an IPv6 (AAAA) record points at {}. Delete that AAAA record, then check again.", found.join(", ")),
            Finding::NoAnswer => "This name points here, but your server did not answer through it. Make sure ports 80 and 443 are open, then check again.".into(),
            Finding::NoCertificate => "Your server is getting the padlock certificate for this name. This usually takes under a minute. If it keeps failing, check that the domain has no CAA record that blocks Let's Encrypt.".into(),
        }
    }
}

/// Classify a failed reachability check from what DNS said.
pub fn classify(resolved: &[IpAddr], expected: Option<IpAddr>) -> Finding {
    if resolved.is_empty() {
        return Finding::NotThereYet;
    }
    if resolved.iter().any(|ip| is_cloudflare_proxy(*ip)) {
        return Finding::CloudflareProxy;
    }
    if let Some(exp) = expected {
        let v4_ok = resolved
            .iter()
            .filter(|ip| ip.is_ipv4())
            .all(|ip| *ip == exp)
            && resolved.contains(&exp);
        if v4_ok {
            let v6: Vec<String> = resolved
                .iter()
                .filter(|ip| ip.is_ipv6())
                .map(|ip| ip.to_string())
                .collect();
            if !v6.is_empty() {
                return Finding::Ipv6Elsewhere { found: v6 };
            }
            return Finding::NoAnswer;
        }
        return Finding::Elsewhere {
            found: resolved.iter().map(|ip| ip.to_string()).collect(),
        };
    }
    Finding::NoAnswer
}

/// Look a name up with the server's own resolver (no outside DNS service).
pub async fn resolve(name: &str) -> Vec<IpAddr> {
    match tokio::time::timeout(Duration::from_secs(8), tokio::net::lookup_host((name, 80))).await {
        Ok(Ok(addrs)) => {
            let mut v: Vec<IpAddr> = addrs.map(|a| a.ip()).collect();
            v.sort();
            v.dedup();
            v
        }
        _ => Vec::new(),
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// Does `scheme://name/.well-known/ebp-check/<token>` answer with the token?
/// Over https this also proves the certificate is valid for the name.
pub async fn token_answers(scheme: &str, name: &str, token: &str) -> bool {
    let url = format!("{scheme}://{name}/.well-known/ebp-check/{token}");
    match client().get(&url).send().await {
        Ok(r) if r.status().is_success() => {
            r.text().await.map(|t| t.trim() == token).unwrap_or(false)
        }
        _ => false,
    }
}

/// The whole pre-check: does `name` reach this server over plain http?
pub async fn precheck(name: &str, token: &str, expected: Option<IpAddr>) -> Finding {
    let resolved = resolve(name).await;
    if resolved.is_empty() {
        return Finding::NotThereYet;
    }
    if resolved.iter().any(|ip| is_cloudflare_proxy(*ip)) {
        return Finding::CloudflareProxy;
    }
    if token_answers("http", name, token).await {
        // An IPv6 record elsewhere would make Let's Encrypt fail later.
        if let Some(exp) = expected {
            if let f @ Finding::Ipv6Elsewhere { .. } = classify(&resolved, Some(exp)) {
                return f;
            }
        }
        return Finding::Ready;
    }
    classify(&resolved, expected)
}

/// Is the check token right and still valid? (Constant-time compare.)
pub fn token_matches(st: &MoveState, given: &str, now: i64) -> bool {
    match &st.token {
        Some(t) if now <= st.token_expires && t.len() == given.len() => {
            t.bytes()
                .zip(given.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
        }
        _ => false,
    }
}

pub fn new_token(st: &mut MoveState, now: i64) -> String {
    let t = crate::util::random_hex(16);
    st.token = Some(t.clone());
    st.token_expires = now + TOKEN_SECS;
    t
}

// ---- The address helper -------------------------------------------------------

fn updates_dir() -> PathBuf {
    PathBuf::from(std::env::var("EBP_UPDATES_DIR").unwrap_or_else(|_| "/updates".into()))
}

/// Is the address helper installed? (It writes `status/address-helper.json`.)
pub fn helper_installed() -> bool {
    updates_dir()
        .join("status")
        .join("address-helper.json")
        .is_file()
}

/// What the web server should serve: the main name first, then the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Serve {
    pub main: String,
    pub others: Vec<String>,
}

/// Ask the helper to serve exactly these names (written to a temporary file,
/// then renamed, so the helper never sees half a request). Returns the id.
pub fn request_serve(serve: &Serve) -> Result<String, AppError> {
    for n in std::iter::once(&serve.main).chain(serve.others.iter()) {
        // The backup sslip.io name is allowed here; anything else must be clean.
        if !is_free_ip_name(n) {
            clean_name(n)?;
        } else if sslip_ip(n).is_none() {
            return Err(AppError::BadRequest(NOT_A_NAME.into()));
        }
    }
    let id = crate::util::random_hex(16);
    let body = serde_json::json!({
        "action": "address",
        "main": serve.main,
        "others": serve.others,
        "id": id,
    });
    let inbox = updates_dir().join("inbox");
    let tmp = inbox.join(format!(".address-{id}.tmp"));
    std::fs::write(&tmp, body.to_string()).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::rename(&tmp, inbox.join("address-request.json"))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(id)
}

/// The helper's answer to a request: `Some(true)` done, `Some(false)` failed,
/// `None` not answered yet.
pub fn helper_result(id: &str) -> Option<bool> {
    let raw =
        std::fs::read_to_string(updates_dir().join("status").join("address-status.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    if v.get("id").and_then(Value::as_str) != Some(id) {
        return None;
    }
    match v.get("state").and_then(Value::as_str) {
        Some("done") => Some(true),
        Some("failed") => Some(false),
        _ => None,
    }
}

// ---- Deciding the next step --------------------------------------------------

/// The server's current addresses, from its settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub main: Option<String>,
    pub backups: Vec<String>,
}

impl Current {
    pub fn from_config(c: &crate::config::Config) -> Self {
        Current {
            main: host_of(&c.public_base_url),
            backups: c.public_aliases.iter().filter_map(|a| host_of(a)).collect(),
        }
    }
    pub fn serves(&self, name: &str) -> bool {
        self.main.as_deref() == Some(name) || self.backups.iter().any(|b| b == name)
    }
}

/// After a restart, see whether a change we asked the helper for has happened,
/// and move the state on. Pure: easy to test.
pub fn reconcile(st: &mut MoveState, cur: &Current, now: i64) {
    let Some(name) = st.name.clone() else { return };
    match st.step {
        Step::ServeRequested if cur.serves(&name) => st.step = Step::Serving,
        Step::MainRequested if cur.main.as_deref() == Some(name.as_str()) => {
            st.step = Step::Main;
            st.switched_at = Some(now);
            st.failures = 0;
            st.last_problem = None;
            st.told.clear();
        }
        Step::RetireRequested
            if cur.backups.is_empty() && cur.main.as_deref() == Some(name.as_str()) =>
        {
            st.step = Step::Retired;
        }
        Step::BackRequested if cur.main.as_deref() != Some(name.as_str()) => {
            st.step = Step::Serving;
            st.switched_at = None;
        }
        _ => {}
    }
}

/// What the watch should do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchAction {
    Nothing,
    /// Check the main name now.
    Check,
    /// Tell the owner the backup will be given up in a few days.
    WarnRetire,
    /// Give up the backup.
    Retire,
}

/// Decide the watch's next action. Pure: easy to test.
pub fn next_watch_action(st: &MoveState, shop_host: Option<&str>, now: i64) -> WatchAction {
    if st.step != Step::Main {
        return WatchAction::Nothing;
    }
    let Some(switched) = st.switched_at else {
        return WatchAction::Nothing;
    };
    let due_check = st.last_check_at.is_none_or(|t| now - t >= WATCH_EVERY_SECS);
    let retire_at = switched + RETIRE_DAYS * DAY;
    let kept = st.keep_backup_until.is_some_and(|t| now < t);
    let all_good = st.failures == 0 && st.last_ok_at.is_some();
    let shop_moved = match (shop_host, st.name.as_deref()) {
        (Some(h), Some(n)) => h == n,
        (None, _) => true, // no shop connected: nothing calls the old name
        _ => false,
    };
    if all_good && shop_moved && !kept && now >= retire_at && !due_check {
        return WatchAction::Retire;
    }
    if all_good
        && shop_moved
        && !kept
        && now >= retire_at - RETIRE_WARN_DAYS * DAY
        && !st.told.iter().any(|t| t == "own_address_backup_soon")
        && !due_check
    {
        return WatchAction::WarnRetire;
    }
    if due_check {
        return WatchAction::Check;
    }
    WatchAction::Nothing
}

/// Record one watch check's outcome.
pub fn record_check(st: &mut MoveState, ok: bool, problem: Option<String>, now: i64) {
    st.last_check_at = Some(now);
    if ok {
        st.last_ok_at = Some(now);
    } else {
        st.failures = st.failures.saturating_add(1);
        st.last_problem = problem;
    }
}

/// The days left in the watch, and until the backup is given up (for Home).
pub fn days_left(st: &MoveState, now: i64) -> Option<(i64, i64)> {
    let s = st.switched_at?;
    let watch = (s + WATCH_DAYS * DAY - now).max(0).div_euclid(DAY);
    let retire_at = st
        .keep_backup_until
        .map(|k| k.max(s + RETIRE_DAYS * DAY))
        .unwrap_or(s + RETIRE_DAYS * DAY);
    Some((watch, (retire_at - now).max(0).div_euclid(DAY)))
}

/// Remember which host name the shop calls with its key (for "the shop has
/// moved"). Writes only when it changes.
pub fn note_shop_host(conn: &Connection, host: &str) -> Result<(), AppError> {
    let h = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    if h.is_empty() || h.len() > 253 {
        return Ok(());
    }
    if settings::get(conn, SHOP_HOST_KEY)?.as_deref() != Some(h.as_str()) {
        settings::set(conn, SHOP_HOST_KEY, &h)?;
    }
    Ok(())
}

// ---- The watch --------------------------------------------------------------

/// One watch round: confirm requested changes, check the main name, warn, and
/// give up the backup when it is time. Runs every few minutes; does real work
/// only when [`next_watch_action`] says so.
pub async fn watch_once(db: &crate::db::Db, cur: &Current) -> Result<(), AppError> {
    let now = now_unix();
    let cur2 = cur.clone();
    let (loaded_step, mut st, shop) = db
        .call(move |c| {
            let mut st = load(c)?;
            let loaded = st.step;
            reconcile(&mut st, &cur2, now);
            Ok((loaded, st, settings::get(c, SHOP_HOST_KEY)?))
        })
        .await?;
    match next_watch_action(&st, shop.as_deref(), now) {
        WatchAction::Nothing => {}
        WatchAction::Check => {
            let name = st.name.clone().unwrap_or_default();
            let token = new_token(&mut st, now);
            let saved = st.clone();
            db.call(move |c| save(c, &saved)).await?;
            let ok = token_answers("https", &name, &token).await;
            let problem = (!ok).then(|| {
                format!("https://{name} did not answer as this server with a valid certificate")
            });
            record_check(&mut st, ok, problem, now_unix());
            if !ok && !st.told.iter().any(|t| t == "own_address_problem") {
                st.told.push("own_address_problem".into());
                db.call(|c| crate::alerts::queue_notice(c, "own_address_problem"))
                    .await?;
            }
        }
        WatchAction::WarnRetire => {
            st.told.push("own_address_backup_soon".into());
            db.call(|c| crate::alerts::queue_notice(c, "own_address_backup_soon"))
                .await?;
        }
        WatchAction::Retire => {
            if helper_installed() {
                if let Some(name) = st.name.clone() {
                    let id = request_serve(&Serve {
                        main: name,
                        others: vec![],
                    })?;
                    st.step = Step::RetireRequested;
                    st.request_id = Some(id);
                    db.call(|c| crate::alerts::queue_notice(c, "own_address_backup_retired"))
                        .await?;
                }
            }
        }
    }
    db.call(move |c| {
        // Only save when nobody changed the move meanwhile (the owner may have
        // pressed Go back while this round was checking).
        if load(c)?.step == loaded_step {
            save(c, &st)?;
        }
        Ok(())
    })
    .await
}

/// The watch loop, started with the server.
pub async fn run(db: crate::db::Db, cur: Current) {
    loop {
        let enabled = db.call(|c| is_enabled(c)).await.unwrap_or(false);
        if enabled {
            if let Err(e) = watch_once(&db, &cur).await {
                tracing::warn!(error = %e, "own-domain watch round failed");
            }
        }
        tokio::time::sleep(Duration::from_secs(600)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_domain_names_are_accepted() {
        assert_eq!(
            clean_name(" HTTPS://Pay.MyShop.com/ ").unwrap(),
            "pay.myshop.com"
        );
        assert_eq!(clean_name("pay.myshop.com.").unwrap(), "pay.myshop.com");
        for bad in [
            "",
            "nodot",
            "203.0.113.7",
            "pay..myshop.com",
            "-pay.myshop.com",
            "pay-.myshop.com",
            "pay.myshop.com;rm -rf",
            "pay myshop.com",
            "pay.myshop.123",
            "pay.my_shop.com",
            "203-0-113-7.sslip.io",
            "x.nip.io",
        ] {
            assert!(clean_name(bad).is_err(), "{bad:?} accepted");
        }
        assert!(
            clean_name(&format!("{}.com", "a".repeat(64))).is_err(),
            "label too long"
        );
    }

    #[test]
    fn sslip_names_give_their_ip() {
        assert_eq!(
            sslip_ip("203-0-113-7.sslip.io"),
            Some("203.0.113.7".parse().unwrap())
        );
        assert_eq!(sslip_ip("pay.myshop.com"), None);
        assert_eq!(sslip_ip("nonsense.sslip.io"), None);
        assert_eq!(
            host_of("https://Pay.Example.com:443/x"),
            Some("pay.example.com".into())
        );
    }

    #[test]
    fn cloudflare_proxy_addresses_are_recognised() {
        for ip in [
            "104.16.1.1",
            "172.67.200.9",
            "162.159.1.1",
            "2606:4700::6810:1",
        ] {
            assert!(is_cloudflare_proxy(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.7", "8.8.8.8", "104.32.0.1", "2001:db8::1"] {
            assert!(!is_cloudflare_proxy(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn a_failed_check_is_explained() {
        let here: IpAddr = "203.0.113.7".parse().unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(classify(&[], Some(here)), Finding::NotThereYet);
        assert_eq!(
            classify(&[ip("104.16.1.1")], Some(here)),
            Finding::CloudflareProxy
        );
        assert_eq!(
            classify(&[ip("198.51.100.4")], Some(here)),
            Finding::Elsewhere {
                found: vec!["198.51.100.4".into()]
            }
        );
        assert_eq!(
            classify(&[here, ip("2001:db8::1")], Some(here)),
            Finding::Ipv6Elsewhere {
                found: vec!["2001:db8::1".into()]
            }
        );
        assert_eq!(classify(&[here], Some(here)), Finding::NoAnswer);
        assert_eq!(
            classify(&[ip("198.51.100.4")], None),
            Finding::NoAnswer,
            "without an expected IP we cannot say more"
        );
    }

    #[test]
    fn the_token_is_single_purpose_and_expires() {
        let mut st = MoveState::default();
        let t = new_token(&mut st, 1000);
        assert!(token_matches(&st, &t, 1000));
        assert!(token_matches(&st, &t, 1000 + TOKEN_SECS));
        assert!(!token_matches(&st, &t, 1001 + TOKEN_SECS), "expired");
        assert!(!token_matches(&st, "nope", 1000));
        assert!(
            !token_matches(&MoveState::default(), "", 1000),
            "no token, nothing matches"
        );
    }

    fn cur(main: &str, backups: &[&str]) -> Current {
        Current {
            main: Some(main.into()),
            backups: backups.iter().map(|b| b.to_string()).collect(),
        }
    }

    #[test]
    fn a_requested_change_is_confirmed_by_the_server_settings_after_restart() {
        let mut st = MoveState {
            step: Step::ServeRequested,
            name: Some("pay.myshop.com".into()),
            ..Default::default()
        };
        reconcile(&mut st, &cur("1-2-3-4.sslip.io", &[]), 10);
        assert_eq!(st.step, Step::ServeRequested, "not served yet");
        reconcile(&mut st, &cur("1-2-3-4.sslip.io", &["pay.myshop.com"]), 10);
        assert_eq!(st.step, Step::Serving);
        st.step = Step::MainRequested;
        reconcile(&mut st, &cur("pay.myshop.com", &["1-2-3-4.sslip.io"]), 20);
        assert_eq!((st.step, st.switched_at), (Step::Main, Some(20)));
        st.step = Step::BackRequested;
        reconcile(&mut st, &cur("1-2-3-4.sslip.io", &["pay.myshop.com"]), 30);
        assert_eq!((st.step, st.switched_at), (Step::Serving, None));
        st.step = Step::RetireRequested;
        reconcile(&mut st, &cur("pay.myshop.com", &[]), 40);
        assert_eq!(st.step, Step::Retired);
    }

    #[test]
    fn the_backup_is_given_up_only_after_30_good_days_with_the_shop_moved() {
        let name = "pay.myshop.com";
        let base = MoveState {
            step: Step::Main,
            name: Some(name.into()),
            switched_at: Some(0),
            last_check_at: Some(30 * DAY - 60),
            last_ok_at: Some(30 * DAY - 60),
            ..Default::default()
        };
        let at = |d: i64| d * DAY;
        assert_eq!(
            next_watch_action(&base, Some(name), at(30)),
            WatchAction::Retire
        );
        assert_eq!(
            next_watch_action(&base, None, at(30)),
            WatchAction::Retire,
            "no shop connected"
        );
        assert_eq!(
            next_watch_action(&base, Some("1-2-3-4.sslip.io"), at(30)),
            WatchAction::Nothing,
            "the shop still calls the backup: keep it"
        );
        let failed = MoveState {
            failures: 1,
            ..base.clone()
        };
        assert_eq!(
            next_watch_action(&failed, Some(name), at(30)),
            WatchAction::Nothing,
            "a failure keeps the backup"
        );
        let kept = MoveState {
            keep_backup_until: Some(at(31)),
            ..base.clone()
        };
        assert_eq!(
            next_watch_action(&kept, Some(name), at(30)),
            WatchAction::Nothing,
            "postponed"
        );
        let early = MoveState {
            last_check_at: Some(at(27)),
            last_ok_at: Some(at(27)),
            ..base.clone()
        };
        assert_eq!(
            next_watch_action(&early, Some(name), at(27) + 60),
            WatchAction::WarnRetire,
            "3 days ahead"
        );
        let warned = MoveState {
            told: vec!["own_address_backup_soon".into()],
            ..early.clone()
        };
        assert_eq!(
            next_watch_action(&warned, Some(name), at(27) + 60),
            WatchAction::Nothing,
            "warned once"
        );
        let stale = MoveState {
            last_check_at: Some(at(29)),
            ..base.clone()
        };
        assert_eq!(
            next_watch_action(&stale, Some(name), at(30)),
            WatchAction::Check,
            "check first, then decide"
        );
        let idle = MoveState {
            step: Step::Serving,
            ..base.clone()
        };
        assert_eq!(
            next_watch_action(&idle, Some(name), at(30)),
            WatchAction::Nothing
        );
        assert_eq!(days_left(&base, at(3)), Some((11, 27)));
    }
}
