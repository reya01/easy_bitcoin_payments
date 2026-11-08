//! Optional phone alerts through ntfy (https://ntfy.sh).
//!
//! The merchant installs the free ntfy app, subscribes to one long random
//! topic name, and EBP posts short, generic messages to it when something
//! needs a human. Messages NEVER contain amounts, addresses, keys or order
//! details; the topic name is the only "secret" and is shown only to the
//! signed-in admin.
//!
//! State lives in the `settings` table:
//! * `alerts_state`  — absent (undecided), `on` or `off`
//! * `alerts_topic`  — the random topic name
//! * `alerts_server` — optional own ntfy server (https), default ntfy.sh
//! * `alert.active.<key>` — unix time the problem was last announced
//!   (`0` = not active), used to alert once and repeat at most daily.
//! * `alerts_activity` — (added with the PHP2Shell hardening, so a redirected or
//!   skimmed shop shows up quickly) how to hear about payments: `daily` (the default; one
//!   short message a day, only on days with payments), `each` (one message per
//!   payment) or `off`. Messages carry counts only, never amounts or addresses.
//! * `alerts_activity_cursor` / `alerts_activity_digest_at` — where the last
//!   per-payment / daily message left off.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::Connection;

use crate::db::Db;
use crate::error::AppError;
use crate::settings;
use crate::util::{now_unix, random_hex};
use crate::wallet::WalletAdapter;

pub const DEFAULT_SERVER: &str = "https://ntfy.sh";
/// Wallet must be unreachable this long before we bother anyone.
pub const WALLET_DOWN_GRACE_SECS: i64 = 5 * 60;
/// Coins this close to expiry that are still around mean renewal is stuck.
pub const COIN_EXPIRY_WARN_SECS: i64 = 3 * 24 * 3600;
/// A still-active problem is repeated at most this often.
pub const REPEAT_SECS: i64 = 24 * 3600;
const CHECK_INTERVAL_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Undecided,
    On,
    Off,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Undecided => "undecided",
            State::On => "on",
            State::Off => "off",
        }
    }
}

/// What the engine currently sees. Collected elsewhere; evaluated purely.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Observation {
    /// Seconds the wallet has been unreachable (`None` = reachable).
    pub wallet_down_for_secs: Option<i64>,
    /// Coins that are close to expiring and still not refreshed.
    pub coins_near_expiry: usize,
    /// The latest automatic withdrawal attempt failed.
    pub last_sweep_failed: bool,
    /// Payments that arrived but could not be matched to an order.
    pub unresolved_payments: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    New,
    Repeat,
    Recovered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub key: &'static str,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    pub urgent: bool,
}

struct Rule {
    key: &'static str,
    active: bool,
    title: &'static str,
    body: &'static str,
    good: &'static str,
}

fn rules(o: &Observation) -> Vec<Rule> {
    vec![
        Rule {
            key: "wallet_down",
            active: o
                .wallet_down_for_secs
                .is_some_and(|s| s >= WALLET_DOWN_GRACE_SECS),
            title: "Your payment server can't reach its wallet",
            body: "New payments may not be recognised until this is fixed. Open your dashboard and check Is everything OK?",
            good: "The wallet connection is back.",
        },
        Rule {
            key: "coins_expiring",
            active: o.coins_near_expiry > 0,
            title: "Some coins are close to expiring",
            body: "They should have been renewed automatically but were not. Open your dashboard to check.",
            good: "Coin renewal is working again.",
        },
        Rule {
            key: "sweep_failed",
            active: o.last_sweep_failed,
            title: "An automatic withdrawal failed",
            body: "Your money is safe, but it did not reach your payout address. Open your dashboard for details.",
            good: "Withdrawals to your payout address are working again.",
        },
        Rule {
            key: "unmatched",
            active: o.unresolved_payments > 0,
            title: "A payment needs your attention",
            body: "A payment arrived that could not be matched to an open order. Open your dashboard, Payments that need you.",
            good: "No payments are waiting for you any more.",
        },
    ]
}

/// Pure alert logic. `announced` maps rule key → time last announced
/// (missing or `<= 0` means not currently announced).
pub fn evaluate(o: &Observation, announced: &HashMap<String, i64>, now: i64) -> Vec<Decision> {
    let mut out = Vec::new();
    for r in rules(o) {
        let last = announced.get(r.key).copied().unwrap_or(0);
        if r.active {
            let kind = if last <= 0 {
                Some(Kind::New)
            } else if now - last >= REPEAT_SECS {
                Some(Kind::Repeat)
            } else {
                None
            };
            if let Some(kind) = kind {
                out.push(Decision {
                    key: r.key,
                    kind,
                    title: r.title.to_string(),
                    body: r.body.to_string(),
                    urgent: true,
                });
            }
        } else if last > 0 {
            out.push(Decision {
                key: r.key,
                kind: Kind::Recovered,
                title: "All good again".to_string(),
                body: r.good.to_string(),
                urgent: false,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- settings

/// How the owner wants to hear about payments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Off,
    Daily,
    Each,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Off => "off",
            Activity::Daily => "daily",
            Activity::Each => "each",
        }
    }
}

/// Default is a daily summary: per-payment messages are too frequent to be
/// the default (the owner can choose them).
pub fn activity(conn: &Connection) -> Result<Activity, AppError> {
    Ok(match settings::get(conn, "alerts_activity")?.as_deref() {
        Some("off") => Activity::Off,
        Some("each") => Activity::Each,
        _ => Activity::Daily,
    })
}

pub fn set_activity(conn: &Connection, mode: &str) -> Result<(), AppError> {
    let mode = match mode {
        "off" | "daily" | "each" => mode,
        _ => return Err(AppError::BadRequest("choose daily, each or off".into())),
    };
    settings::set(conn, "alerts_activity", mode)?;
    // Start counting from now so switching never replays old payments.
    let now = now_unix().to_string();
    settings::set(conn, "alerts_activity_cursor", &now)?;
    settings::set(conn, "alerts_activity_digest_at", &now)
}

/// Orders this server marked paid at or after `since` (test payments
/// excluded). "At or after" so a payment landing in the very second a summary
/// was cut is never lost; the worst case is one rare repeat count.
pub fn paid_since(conn: &Connection, since: i64) -> Result<i64, AppError> {
    let mut stmt = conn.prepare(
        "SELECT metadata_json FROM invoices
         WHERE status IN ('paid','overpaid') AND paid_at IS NOT NULL AND paid_at >= ?1",
    )?;
    let metas = stmt
        .query_map([since], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(metas
        .iter()
        .filter(|m| !crate::invoice::is_test_invoice(m))
        .count() as i64)
}

/// A message about payments, or nothing. Pure.
///
/// * `each`: one message when something was paid since the last one.
/// * `daily`: at most one a day, and only if something was paid in that day
///   (a quiet day sends nothing).
pub fn activity_message(
    mode: Activity,
    now: i64,
    last_digest_at: i64,
    paid_since_cursor: i64,
    paid_since_digest: i64,
) -> Option<(String, String)> {
    let check = "Open your own wallet and check that the same payments arrived.";
    match mode {
        Activity::Off => None,
        Activity::Each if paid_since_cursor > 0 => Some((
            "A payment arrived".to_string(),
            format!("{check} (No amounts are sent in alerts.)"),
        )),
        Activity::Daily if now - last_digest_at >= 24 * 3600 && paid_since_digest > 0 => Some((
            format!(
                "{} payment{} in the last day",
                paid_since_digest,
                if paid_since_digest == 1 { "" } else { "s" }
            ),
            format!("{check} (No amounts are sent in alerts.)"),
        )),
        _ => None,
    }
}

pub fn state(conn: &Connection) -> Result<State, AppError> {
    Ok(match settings::get(conn, "alerts_state")?.as_deref() {
        Some("on") => State::On,
        Some("off") => State::Off,
        _ => State::Undecided,
    })
}

pub fn topic(conn: &Connection) -> Result<Option<String>, AppError> {
    Ok(settings::get(conn, "alerts_topic")?.filter(|t| !t.is_empty()))
}

pub fn server(conn: &Connection) -> Result<String, AppError> {
    if let Some(s) = settings::get(conn, "alerts_server")?.filter(|s| !s.is_empty()) {
        return Ok(s);
    }
    if let Ok(s) = std::env::var("EBP_ALERTS_SERVER") {
        if !s.trim().is_empty() {
            return Ok(s.trim().to_string());
        }
    }
    Ok(DEFAULT_SERVER.to_string())
}

/// Accept only https (plain http only for local development, when
/// `EBP_WEBHOOK_ALLOW_PRIVATE=1`), and never loopback / private / link-local
/// addresses: an alerts server the admin types in must not become a way to
/// poke at services inside the host's network.
pub fn validate_server(s: &str) -> Result<String, AppError> {
    validate_server_with(s, crate::webhook::allow_private_from_env())
}

pub fn validate_server_with(s: &str, allow_private: bool) -> Result<String, AppError> {
    let s = s.trim().trim_end_matches('/');
    let scheme_ok = s.starts_with("https://") || (allow_private && s.starts_with("http://"));
    if !scheme_ok || s.len() < 9 || s.chars().any(|c| c.is_whitespace()) {
        return Err(AppError::BadRequest(
            "the alerts server address must start with https://".into(),
        ));
    }
    crate::webhook::validate_webhook_url(s, allow_private).map_err(|e| {
        AppError::BadRequest(format!("that alerts server address is not allowed: {e}"))
    })?;
    Ok(s.to_string())
}

fn new_topic() -> String {
    format!("ebp-{}", random_hex(16))
}

/// Turn alerts on (keeps an existing topic so the phone stays subscribed).
pub fn enable(conn: &Connection, own_server: Option<&str>) -> Result<(), AppError> {
    if topic(conn)?.is_none() {
        settings::set(conn, "alerts_topic", &new_topic())?;
    }
    match own_server.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => settings::set(conn, "alerts_server", &validate_server(s)?)?,
        None => settings::set(conn, "alerts_server", "")?,
    }
    // Turning alerts back on after a pause starts counting from now, so the
    // first summary never reports payments from the time they were off.
    if settings::get(conn, "alerts_state")?.as_deref() != Some("on") {
        let now = now_unix().to_string();
        settings::set(conn, "alerts_activity_cursor", &now)?;
        settings::set(conn, "alerts_activity_digest_at", &now)?;
    }
    settings::set(conn, "alerts_state", "on")
}

pub fn disable(conn: &Connection) -> Result<(), AppError> {
    settings::set(conn, "alerts_state", "off")
}

fn load_announced(conn: &Connection) -> Result<HashMap<String, i64>, AppError> {
    let mut m = HashMap::new();
    for r in rules(&Observation::default()) {
        let v = settings::get(conn, &format!("alert.active.{}", r.key))?
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        m.insert(r.key.to_string(), v);
    }
    Ok(m)
}

// ------------------------------------------------------------------ sending

/// Post one message to ntfy. Errors are returned, never panicked on.
pub async fn send(
    server: &str,
    topic: &str,
    title: &str,
    body: &str,
    urgent: bool,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // Never follow a redirect: it could lead somewhere validate_server refused.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{}/{}", server.trim_end_matches('/'), topic))
        .header("Title", title)
        .header("Priority", if urgent { "high" } else { "default" })
        .header(
            "Tags",
            if urgent {
                "warning"
            } else {
                "white_check_mark"
            },
        )
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("the alerts server answered {}", resp.status()))
    }
}

/// `POST /admin/alerts/test` helper.
pub async fn send_test(db: &Db) -> Result<(), AppError> {
    let (server, topic) = db
        .call(|c| Ok((validate_server(&server(c)?)?, topic(c)?)))
        .await?;
    let topic = topic.ok_or_else(|| AppError::BadRequest("turn phone alerts on first".into()))?;
    send(
        &server,
        &topic,
        "Test alert from your payment server",
        "If you can read this, phone alerts are working.",
        false,
    )
    .await
    .map_err(|e| AppError::BadRequest(format!("could not send the test alert: {e}")))
}

// ------------------------------------------------------------------- engine

fn observe_db(conn: &Connection, now: i64) -> Result<(bool, i64), AppError> {
    let failed: bool = conn
        .query_row(
            "SELECT status = 'failed' FROM settlements
             WHERE kind IN ('sweep_ark','sweep_onchain')
             ORDER BY created_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other),
        })?;
    let _ = now;
    let unresolved: i64 = conn.query_row(
        "SELECT COUNT(*) FROM unmatched_payments WHERE resolved_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    Ok((failed, unresolved))
}

/// Background loop; does nothing unless the merchant turned alerts on.
pub async fn run(db: Db, wallet: Arc<dyn WalletAdapter>) {
    let mut down_since: Option<i64> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(CHECK_INTERVAL_SECS)).await;
        if let Err(e) = tick(&db, &wallet, &mut down_since).await {
            tracing::warn!(error = %e, "phone alerts check failed");
        }
    }
}

async fn tick(
    db: &Db,
    wallet: &Arc<dyn WalletAdapter>,
    down_since: &mut Option<i64>,
) -> Result<(), AppError> {
    let now = now_unix();
    let w = wallet.clone();
    let health = tokio::task::spawn_blocking(move || w.health())
        .await
        .map_err(|_| AppError::Internal("wallet check failed".into()))?;
    if health.reachable {
        *down_since = None;
    } else if down_since.is_none() {
        *down_since = Some(now);
    }

    if db.call(|c| state(c)).await? != State::On {
        return Ok(());
    }

    let near = if health.reachable {
        let w = wallet.clone();
        tokio::task::spawn_blocking(move || w.vtxos())
            .await
            .map_err(|_| AppError::Internal("wallet check failed".into()))?
            .map(|v| {
                v.iter()
                    .filter(|x| x.expires_at > 0 && x.expires_at - now < COIN_EXPIRY_WARN_SECS)
                    .count()
            })
            .unwrap_or(0)
    } else {
        0
    };

    let ds = *down_since;
    let (decisions, server, topic) = db
        .call(move |c| {
            let (failed, unresolved) = observe_db(c, now)?;
            let obs = Observation {
                wallet_down_for_secs: ds.map(|t| now - t),
                coins_near_expiry: near,
                last_sweep_failed: failed,
                unresolved_payments: unresolved,
            };
            let d = evaluate(&obs, &load_announced(c)?, now);
            Ok((d, validate_server(&server(c)?), topic(c)?))
        })
        .await?;
    let Some(topic) = topic else { return Ok(()) };
    let server = server?;

    for d in decisions {
        match send(&server, &topic, &d.title, &d.body, d.urgent).await {
            Ok(()) => {
                let value = if d.kind == Kind::Recovered { 0 } else { now };
                let key = format!("alert.active.{}", d.key);
                db.call(move |c| settings::set(c, &key, &value.to_string()))
                    .await?;
            }
            // Not recorded, so it is retried on the next check.
            Err(e) => tracing::warn!(error = %e, "could not send a phone alert"),
        }
    }

    // Payment activity (daily summary by default; the owner can pick each
    // payment, or none).
    let (now, mode, digest_at, new_since_cursor, new_since_digest) = db
        .call(move |c| {
            // Sampled in the same step as the counts, so the cursor we save
            // afterwards matches exactly what was counted.
            let now = now_unix();
            let mode = activity(c)?;
            let get = |k: &str, c: &Connection| -> Result<i64, AppError> {
                Ok(settings::get(c, k)?
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0))
            };
            let mut cursor = get("alerts_activity_cursor", c)?;
            let mut digest_at = get("alerts_activity_digest_at", c)?;
            // First run: start from now, never from the beginning of history.
            if cursor <= 0 {
                cursor = now;
                settings::set(c, "alerts_activity_cursor", &now.to_string())?;
            }
            if digest_at <= 0 {
                digest_at = now;
                settings::set(c, "alerts_activity_digest_at", &now.to_string())?;
            }
            Ok((
                now,
                mode,
                digest_at,
                paid_since(c, cursor)?,
                paid_since(c, digest_at)?,
            ))
        })
        .await?;
    if let Some((title, body)) =
        activity_message(mode, now, digest_at, new_since_cursor, new_since_digest)
    {
        match send(&server, &topic, &title, &body, false).await {
            Ok(()) => {
                let key = if mode == Activity::Each {
                    "alerts_activity_cursor"
                } else {
                    "alerts_activity_digest_at"
                };
                db.call(move |c| settings::set(c, key, &now.to_string()))
                    .await?;
            }
            Err(e) => tracing::warn!(error = %e, "could not send a payment summary"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashMap<String, i64> {
        HashMap::new()
    }

    #[test]
    fn payment_summaries_are_daily_by_default_and_quiet_on_empty_days() {
        let day = 24 * 3600;
        // Daily: nothing until a day has passed, nothing on a day without payments.
        assert!(activity_message(Activity::Daily, day - 1, 0, 5, 5).is_none());
        assert!(activity_message(Activity::Daily, day, 0, 0, 0).is_none());
        let (title, body) = activity_message(Activity::Daily, day, 0, 0, 3).unwrap();
        assert_eq!(title, "3 payments in the last day");
        assert!(body.contains("check that the same payments arrived"));
        assert_eq!(
            activity_message(Activity::Daily, day, 0, 0, 1).unwrap().0,
            "1 payment in the last day"
        );
        // Each: any new payment, any time.
        assert!(activity_message(Activity::Each, 10, 0, 1, 1).is_some());
        assert!(activity_message(Activity::Each, 10, 0, 0, 5).is_none());
        // Off: never. No amounts in any message.
        assert!(activity_message(Activity::Off, day, 0, 9, 9).is_none());
        let (_, b) = activity_message(Activity::Each, 10, 0, 2, 2).unwrap();
        assert!(!b.chars().any(|c| c.is_ascii_digit()), "{b}");
    }

    #[test]
    fn quiet_when_everything_is_fine() {
        assert!(evaluate(&Observation::default(), &none(), 1000).is_empty());
    }

    #[test]
    fn wallet_down_waits_for_the_grace_period() {
        let mut o = Observation {
            wallet_down_for_secs: Some(WALLET_DOWN_GRACE_SECS - 1),
            ..Default::default()
        };
        assert!(evaluate(&o, &none(), 1000).is_empty());
        o.wallet_down_for_secs = Some(WALLET_DOWN_GRACE_SECS);
        let d = evaluate(&o, &none(), 1000);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].key, "wallet_down");
        assert_eq!(d[0].kind, Kind::New);
    }

    #[test]
    fn a_problem_is_announced_once_then_repeats_daily() {
        let o = Observation {
            coins_near_expiry: 2,
            ..Default::default()
        };
        let mut a = none();
        assert_eq!(evaluate(&o, &a, 10_000)[0].kind, Kind::New);
        a.insert("coins_expiring".into(), 10_000);
        assert!(evaluate(&o, &a, 10_000 + REPEAT_SECS - 1).is_empty());
        let d = evaluate(&o, &a, 10_000 + REPEAT_SECS);
        assert_eq!(d[0].kind, Kind::Repeat);
    }

    #[test]
    fn recovery_is_announced_once() {
        let mut a = none();
        a.insert("sweep_failed".into(), 5_000);
        let d = evaluate(&Observation::default(), &a, 6_000);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, Kind::Recovered);
        assert!(!d[0].urgent);
        a.insert("sweep_failed".into(), 0);
        assert!(evaluate(&Observation::default(), &a, 7_000).is_empty());
    }

    #[test]
    fn messages_never_carry_amounts_or_addresses() {
        let o = Observation {
            wallet_down_for_secs: Some(10_000),
            coins_near_expiry: 3,
            last_sweep_failed: true,
            unresolved_payments: 4,
        };
        for d in evaluate(&o, &none(), 1) {
            let t = format!("{} {}", d.title, d.body).to_lowercase();
            assert!(!t.contains("sats") && !t.contains("bc1") && !t.contains("ark1"));
            assert!(!t.chars().any(|c| c.is_ascii_digit()), "no numbers: {t}");
        }
    }

    #[test]
    fn server_must_be_https_and_public() {
        assert!(validate_server_with("https://ntfy.example.com/", false).is_ok());
        assert!(validate_server_with("http://example.com", false).is_err());
        assert!(validate_server_with("https://exa mple.com", false).is_err());
        for bad in [
            "https://127.0.0.1",
            "https://localhost",
            "https://169.254.169.254/latest",
            "https://10.0.0.5:8080",
            "https://[::1]",
            "https://user:pw@ntfy.example.com",
        ] {
            assert!(
                validate_server_with(bad, false).is_err(),
                "{bad} must be refused"
            );
        }
        // Local development only (EBP_WEBHOOK_ALLOW_PRIVATE=1).
        assert!(validate_server_with("http://127.0.0.1:9999", true).is_ok());
    }

    #[tokio::test]
    async fn enable_disable_and_topic_is_stable() {
        let db = crate::db::Db::open(":memory:").unwrap();
        db.call(|conn| {
            assert_eq!(state(conn)?, State::Undecided);
            enable(conn, None)?;
            let t1 = topic(conn)?.unwrap();
            assert!(t1.starts_with("ebp-") && t1.len() >= 30);
            disable(conn)?;
            assert_eq!(state(conn)?, State::Off);
            enable(conn, None)?;
            assert_eq!(topic(conn)?.unwrap(), t1);
            assert_eq!(server(conn)?, DEFAULT_SERVER);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn send_posts_title_and_body_to_the_topic() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        send(&format!("http://{addr}"), "ebp-abc", "Hello", "World", true)
            .await
            .unwrap();
        let req = h.join().unwrap().to_lowercase();
        assert!(req.starts_with("post /ebp-abc"));
        assert!(req.contains("title: hello"));
        assert!(req.contains("priority: high"));
        assert!(req.contains("world"));
    }
}
