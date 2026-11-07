//! "Is a newer version out?" — for the admin page's update notice.
//!
//! EBP NEVER updates itself. This only reads the project's public releases
//! page (one anonymous GET, cached for hours, nothing about the merchant is
//! sent) so the admin page can say "Update available" and how to apply it.
//! Switch it off with `EBP_UPDATE_CHECK=false`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const LATEST_URL: &str =
    "https://api.github.com/repos/reya01/easy_bitcoin_payments/releases/latest";
const OK_TTL: Duration = Duration::from_secs(6 * 3600);
const FAIL_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Latest {
    pub version: String,
    pub url: String,
    /// The release title carries `[security]`.
    pub security: bool,
}

type Cache = Option<(Instant, Option<Latest>)>;
static CACHE: Mutex<Cache> = Mutex::new(None);

/// `major.minor.patch` (a leading `v` and any `-prerelease`/`+build` suffix
/// are ignored); `None` if it does not look like a version.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let v = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(v)
}

/// Is `latest` newer than `current`? A pre-release build (`0.1.0-rc1`) is
/// older than the final release with the same numbers (`0.1.0`), so people
/// running a release candidate are told when the real release exists.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) if l != c => l > c,
        (Some(_), Some(_)) => {
            let pre = |v: &str| v.trim().trim_start_matches('v').contains('-');
            pre(current) && !pre(latest)
        }
        _ => false,
    }
}

/// Only github.com release pages are ever linked from the admin page.
fn safe_release_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
}

pub fn parse_release(v: &Value) -> Option<Latest> {
    if v.get("draft").and_then(Value::as_bool) == Some(true)
        || v.get("prerelease").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    parse_version(tag)?;
    let url = v.get("html_url")?.as_str()?;
    if !safe_release_url(url) {
        return None;
    }
    let title = v.get("name").and_then(Value::as_str).unwrap_or("");
    Some(Latest {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
        security: title.to_ascii_lowercase().contains("[security]"),
    })
}

async fn fetch() -> Option<Latest> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("easy-bitcoin-payments-update-check")
        .build()
        .ok()?;
    let resp = client
        .get(LATEST_URL)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    parse_release(&resp.json::<Value>().await.ok()?)
}

/// The newest published release, cached.
pub async fn latest() -> Option<Latest> {
    if let Ok(g) = CACHE.lock() {
        if let Some((at, value)) = g.as_ref() {
            let ttl = if value.is_some() { OK_TTL } else { FAIL_TTL };
            if at.elapsed() < ttl {
                return value.clone();
            }
        }
    }
    let value = fetch().await;
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((Instant::now(), value.clone()));
    }
    value
}

/// Which "how to update" text the admin page should show.
pub fn host_kind() -> &'static str {
    if std::env::var("RENDER").as_deref() == Ok("true") {
        "render"
    } else {
        "docker"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        // A release candidate is older than its final release, never newer.
        assert!(is_newer("0.1.0", "0.1.0-rc1"));
        assert!(is_newer("v0.1.0", "v0.1.0-rc2"));
        assert!(!is_newer("0.1.0-rc2", "0.1.0"));
        assert!(!is_newer("0.1.0-rc2", "0.1.0-rc1"));
        assert!(!is_newer("0.1.0", "0.1.0+build5"));
        assert_eq!(parse_version("v1.2.3-rc1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
    }

    #[test]
    fn release_parsing_is_strict() {
        let ok = json!({"tag_name":"v0.2.0","name":"v0.2.0 [Security] fixes","html_url":"https://github.com/reya01/easy_bitcoin_payments/releases/tag/v0.2.0"});
        let l = parse_release(&ok).unwrap();
        assert_eq!(l.version, "0.2.0");
        assert!(l.security);
        // Drafts, prereleases, odd tags and foreign links are ignored.
        assert!(parse_release(
            &json!({"tag_name":"v0.2.0","draft":true,"html_url":"https://github.com/x"})
        )
        .is_none());
        assert!(parse_release(
            &json!({"tag_name":"v0.2.0","prerelease":true,"html_url":"https://github.com/x"})
        )
        .is_none());
        assert!(
            parse_release(&json!({"tag_name":"nightly","html_url":"https://github.com/x"}))
                .is_none()
        );
        assert!(
            parse_release(&json!({"tag_name":"v0.2.0","html_url":"https://evil.example/x"}))
                .is_none()
        );
    }
}
