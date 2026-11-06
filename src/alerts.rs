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

/// Accept only https (plain http only for a local test server).
pub fn validate_server(s: &str) -> Result<String, AppError> {
    let s = s.trim().trim_end_matches('/');
    let local = s.starts_with("http://127.0.0.1") || s.starts_with("http://localhost");
    if !(s.starts_with("https://") || local) || s.len() < 9 || s.chars().any(|c| c.is_whitespace())
    {
        return Err(AppError::BadRequest(
            "the alerts server address must start with https://".into(),
        ));
    }
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
    let (server, topic) = db.call(|c| Ok((server(c)?, topic(c)?))).await?;
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
            Ok((d, server(c)?, topic(c)?))
        })
        .await?;
    let Some(topic) = topic else { return Ok(()) };

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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashMap<String, i64> {
        HashMap::new()
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
    fn server_must_be_https() {
        assert!(validate_server("https://ntfy.example.com/").is_ok());
        assert!(validate_server("http://example.com").is_err());
        assert!(validate_server("https://exa mple.com").is_err());
        assert!(validate_server("http://127.0.0.1:9999").is_ok());
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
