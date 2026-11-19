//! "Is a newer version out?" — for the admin page's update notice.
//!
//! EBP NEVER updates itself. This only reads the project's public releases
//! page (one anonymous GET, cached for hours, nothing about the merchant is
//! sent) so the admin page can say "Update available" and how to apply it.
//! Switch it off with `EBP_UPDATE_CHECK=false`.
//!
//! It also talks to the optional update helper (`scripts/update-helper.sh`)
//! through two folders: EBP drops a request in `/updates/inbox` after the owner
//! pressed a button on the Home tab, and reads the helper's answer in
//! `/updates/status` (mounted read-only). EBP never runs the update itself.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/reya01/easy_bitcoin_payments";
/// The newest releases, release candidates included (every release is one today).
const RELEASES_URL: &str =
    "https://api.github.com/repos/reya01/easy_bitcoin_payments/releases?per_page=20";
const OK_TTL: Duration = Duration::from_secs(6 * 3600);
const FAIL_TTL: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Latest {
    pub version: String,
    pub url: String,
    /// The release title carries `[security]`.
    pub security: bool,
    /// A release candidate.
    pub prerelease: bool,
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

/// A version EBP will show or ask the helper for: `1.2.3` or `1.2.3-rc1`
/// (letters, digits and dots after the dash), at most 64 characters, an
/// optional leading `v`. The same rule as `VERSION_RE` in `scripts/update.sh`.
pub fn valid_version(s: &str) -> bool {
    let v = s.strip_prefix('v').unwrap_or(s);
    if v.is_empty() || v.len() > 64 {
        return false;
    }
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    if nums.len() != 3
        || nums
            .iter()
            .any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    match pre {
        None => true,
        Some(p) => !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'),
    }
}

/// Compare two versions the way `version_cmp` in `scripts/update.sh` does:
/// numbers first; a version with a `-something` part is older than the same
/// numbers without it; `rc10` is newer than `rc9`. `None` if either is not a
/// valid version.
pub fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    if !valid_version(a) || !valid_version(b) {
        return None;
    }
    let (ca, cb) = (parse_version(a)?, parse_version(b)?);
    if ca != cb {
        return Some(ca.cmp(&cb));
    }
    let pre = |v: &str| {
        let v = v.strip_prefix('v').unwrap_or(v);
        v.split_once('-').map(|(_, p)| p.to_string())
    };
    match (pre(a), pre(b)) {
        (None, None) => Some(Ordering::Equal),
        (None, Some(_)) => Some(Ordering::Greater),
        (Some(_), None) => Some(Ordering::Less),
        (Some(pa), Some(pb)) => {
            let xa: Vec<&str> = pa.split('.').collect();
            let xb: Vec<&str> = pb.split('.').collect();
            for i in 0..xa.len().max(xb.len()) {
                let (sa, sb) = match (xa.get(i), xb.get(i)) {
                    (None, _) => return Some(Ordering::Less),
                    (_, None) => return Some(Ordering::Greater),
                    (Some(sa), Some(sb)) => (*sa, *sb),
                };
                let na = sa.bytes().all(|c| c.is_ascii_digit());
                let nb = sb.bytes().all(|c| c.is_ascii_digit());
                let o = match (na, nb) {
                    (true, true) => num(sa).cmp(&num(sb)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    (false, false) => {
                        let wa = sa.trim_end_matches(|c: char| c.is_ascii_digit());
                        let wb = sb.trim_end_matches(|c: char| c.is_ascii_digit());
                        let da = &sa[wa.len()..];
                        let db = &sb[wb.len()..];
                        wa.cmp(wb).then(num(da).cmp(&num(db)))
                    }
                };
                if o != Ordering::Equal {
                    return Some(o);
                }
            }
            Some(Ordering::Equal)
        }
    }
}

fn num(s: &str) -> u64 {
    s.parse().unwrap_or(0)
}

/// Is `latest` newer than `current`? A release candidate (`0.1.0-rc1`) is
/// older than the final release with the same numbers (`0.1.0`) and older
/// than a later candidate (`0.1.0-rc2`).
pub fn is_newer(latest: &str, current: &str) -> bool {
    compare_versions(latest, current) == Some(Ordering::Greater)
}

/// Only github.com release pages are ever linked from the admin page.
fn safe_release_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
}

pub fn parse_release(v: &Value) -> Option<Latest> {
    if v.get("draft").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let tag = v.get("tag_name")?.as_str()?;
    if !valid_version(tag) {
        return None;
    }
    let url = v.get("html_url")?.as_str()?;
    if !safe_release_url(url) {
        return None;
    }
    let title = v.get("name").and_then(Value::as_str).unwrap_or("");
    Some(Latest {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
        security: title.to_ascii_lowercase().contains("[security]"),
        prerelease: v.get("prerelease").and_then(Value::as_bool) == Some(true),
    })
}

/// The newest release in a GitHub releases list, by version (not by date).
pub fn newest_release(list: &Value) -> Option<Latest> {
    list.as_array()?
        .iter()
        .filter_map(parse_release)
        .max_by(|a, b| compare_versions(&a.version, &b.version).unwrap_or(Ordering::Equal))
}

async fn fetch() -> Option<Latest> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("easy-bitcoin-payments-update-check")
        .build()
        .ok()?;
    let resp = client
        .get(RELEASES_URL)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    newest_release(&resp.json::<Value>().await.ok()?)
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

// ---------------------------------------------------------------------------
// The update helper's two folders.

/// Where the helper's folders are mounted (`EBP_UPDATES_DIR`, default `/updates`).
fn updates_dir() -> PathBuf {
    PathBuf::from(std::env::var("EBP_UPDATES_DIR").unwrap_or_else(|_| "/updates".into()))
}

/// What the helper last reported (`status/status.json`), every field checked.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct HelperStatus {
    /// checking, ready, refused, updating, done or failed
    pub state: String,
    /// Why, in one word: ok, older, same_version, signature_failed, digest_changed, ...
    pub code: String,
    pub request_id: String,
    pub running: String,
    pub version: String,
    pub digest: String,
    pub commit: String,
    pub run_url: String,
    pub at: i64,
}

fn read_small(path: &std::path::Path) -> Option<Value> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    f.take(4096).read_to_end(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn hex_of_len(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parse the helper's status, dropping any field that does not match its pattern.
pub fn parse_status(v: &Value) -> Option<HelperStatus> {
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let state = get("state");
    if !["checking", "ready", "refused", "updating", "done", "failed"].contains(&state.as_str()) {
        return None;
    }
    let code = get("code");
    let code = if !code.is_empty()
        && code.len() <= 40
        && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
    {
        code
    } else {
        String::new()
    };
    let checked = |k: &str, ok: fn(&str) -> bool| {
        let s = get(k);
        if ok(&s) {
            s
        } else {
            String::new()
        }
    };
    let run_url = get("run_url");
    let run_ok = run_url
        .strip_prefix(&format!("{REPO_URL}/actions/runs/"))
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b'/' || b.is_ascii_lowercase())
        });
    Some(HelperStatus {
        state,
        code,
        request_id: checked("request_id", |s| hex_of_len(s, 16, 64)),
        running: checked("running", valid_version),
        version: checked("version", valid_version),
        digest: checked("digest", |s| {
            s.strip_prefix("sha256:")
                .is_some_and(|h| hex_of_len(h, 64, 64))
        }),
        commit: checked("commit", |s| hex_of_len(s, 40, 40)),
        run_url: if run_ok { run_url } else { String::new() },
        at: v.get("at").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// The helper's own version if it is installed (it writes `status/helper.json`).
pub fn helper_installed() -> Option<i64> {
    read_small(&updates_dir().join("status").join("helper.json"))?
        .get("helper_version")
        .and_then(Value::as_i64)
}

/// The helper's last answer, if any.
pub fn helper_status() -> Option<HelperStatus> {
    parse_status(&read_small(
        &updates_dir().join("status").join("status.json"),
    )?)
}

/// Drop a request for the helper. `digest` is set only for "apply". Written to a
/// temporary file and renamed, so the helper never sees half a request.
/// Returns the request id.
pub fn write_request(action: &str, version: &str, digest: Option<&str>) -> std::io::Result<String> {
    use rand::RngCore;
    let mut raw = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    let id = hex::encode(raw);
    let mut body = serde_json::json!({ "action": action, "version": version, "id": id });
    if let Some(d) = digest {
        body["digest"] = Value::String(d.to_string());
    }
    let inbox = updates_dir().join("inbox");
    let tmp = inbox.join(format!(".request-{id}.tmp"));
    std::fs::write(&tmp, body.to_string())?;
    std::fs::rename(&tmp, inbox.join("request.json"))?;
    Ok(id)
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
        // A later release candidate is newer, and rc10 comes after rc9.
        assert!(is_newer("0.1.0-rc2", "0.1.0-rc1"));
        assert!(is_newer("0.2.1-rc10", "0.2.1-rc9"));
        assert!(is_newer("0.2.2-rc1", "0.2.1-rc2"));
        assert!(!is_newer("0.2.1-rc1", "0.2.1-rc1"));
        assert!(!is_newer("0.2.0-rc5", "0.2.1-rc1"));
        // Build metadata and odd strings are not versions here.
        assert!(!is_newer("0.1.0", "0.1.0+build5"));
        assert!(!is_newer("0.1.0-rc1; reboot", "0.0.1"));
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
        assert!(!l.prerelease);
        // Drafts, odd tags and foreign links are ignored; release candidates are kept.
        assert!(parse_release(
            &json!({"tag_name":"v0.2.0","draft":true,"html_url":"https://github.com/x"})
        )
        .is_none());
        let rc = parse_release(
            &json!({"tag_name":"v0.2.1-rc2","prerelease":true,"html_url":"https://github.com/x"}),
        )
        .unwrap();
        assert!(rc.prerelease);
        assert_eq!(rc.version, "0.2.1-rc2");
        assert!(
            parse_release(&json!({"tag_name":"nightly","html_url":"https://github.com/x"}))
                .is_none()
        );
        assert!(
            parse_release(&json!({"tag_name":"v0.2.0","html_url":"https://evil.example/x"}))
                .is_none()
        );
    }

    #[test]
    fn newest_release_is_chosen_by_version_not_date() {
        let list = json!([
            {"tag_name":"v0.2.1-rc1","prerelease":true,"html_url":"https://github.com/a"},
            {"tag_name":"v0.2.2-rc1","prerelease":true,"html_url":"https://github.com/b"},
            {"tag_name":"v0.2.1-rc2","prerelease":true,"html_url":"https://github.com/c"},
            {"tag_name":"v9.9.9","draft":true,"html_url":"https://github.com/d"},
            {"tag_name":"nightly","html_url":"https://github.com/e"}
        ]);
        assert_eq!(newest_release(&list).unwrap().version, "0.2.2-rc1");
    }

    #[test]
    fn helper_status_fields_are_checked() {
        let good = json!({"state":"ready","code":"ok","request_id":"0123456789abcdef","running":"0.2.1-rc2",
            "version":"0.2.2-rc1","digest":format!("sha256:{}", "a".repeat(64)),"commit":"b".repeat(40),
            "run_url":"https://github.com/reya01/easy_bitcoin_payments/actions/runs/123/attempts/1","at":5});
        let s = parse_status(&good).unwrap();
        assert_eq!(s.state, "ready");
        assert_eq!(s.version, "0.2.2-rc1");
        assert_eq!(s.commit.len(), 40);
        assert!(s.run_url.ends_with("/attempts/1"));
        // An unknown state is no status at all; bad fields are dropped, not shown.
        assert!(parse_status(&json!({"state":"pwned"})).is_none());
        let bad = json!({"state":"refused","code":"<script>","version":"1.0.0; rm","digest":"sha256:zz",
            "commit":"xyz","run_url":"https://evil.example/actions/runs/1"});
        let s = parse_status(&bad).unwrap();
        assert_eq!(
            (s.code.as_str(), s.version.as_str(), s.digest.as_str()),
            ("", "", "")
        );
        assert_eq!((s.commit.as_str(), s.run_url.as_str()), ("", ""));
    }

    #[test]
    fn versions_are_validated_like_update_sh() {
        for ok in ["0.2.1", "v0.2.1", "0.2.1-rc1", "1.0.0-rc.1"] {
            assert!(valid_version(ok), "{ok}");
        }
        for bad in [
            "",
            "0.2",
            "0.2.1.4",
            "0.2.1-",
            "0.2.1-rc 1",
            "0.2.1-rc1;x",
            "latest",
            "0.2.x",
        ] {
            assert!(!valid_version(bad), "{bad}");
        }
    }
}
