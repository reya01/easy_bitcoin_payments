//! Update from the Home tab, through the optional update helper
//! (`scripts/update-helper.sh`). EBP never updates itself and never gets
//! control of the server: these two endpoints only drop a request file that
//! the helper, running outside EBP, reads. All the safety checks (signature
//! from this project's release workflow, the tag's commit, never going
//! backwards, the exact digest) run in the helper, through `scripts/update.sh`.
//!
//! The owner decides twice:
//! * `POST /admin/update/check` `{version}` — first decision: "Start update".
//!   The helper checks the release and changes nothing; the Home tab then
//!   shows its answer (from `GET /api/v1/admin/version`, `helper_status`).
//! * `POST /admin/update/apply` `{version, digest, password, code}` — second
//!   decision, after seeing the signature result and both versions. Needs the
//!   admin password and, when two-step sign-in is on, a 6-digit code. Only the
//!   digest the owner was shown can be installed.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::error::AppError;
use crate::routes::admin_session::{check_second_step, require_session};
use crate::routes::AppState;
use crate::updates;

fn audit(conn: &rusqlite::Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

/// A check older than this is not what the owner is looking at any more.
const CHECK_MAX_AGE_SECS: i64 = 30 * 60;

/// Does the helper's last answer show exactly this version and digest, fresh
/// and ready? Only then may "Update now" be approved.
fn check_is_current(
    shown: Option<&updates::HelperStatus>,
    version: &str,
    digest: &str,
    now: i64,
) -> bool {
    shown.is_some_and(|s| {
        s.state == "ready"
            && s.code == "ok"
            && s.version == version
            && !s.digest.is_empty()
            && s.digest == digest
            && s.at > 0
            && now - s.at <= CHECK_MAX_AGE_SECS
    })
}

fn not_installed() -> AppError {
    AppError::Conflict(
        "The update helper is not installed on this server, so updates still run from the Console. See the Update card for the one line that installs it.".into(),
    )
}

/// The version must look like a version and be newer than this server's.
fn check_version(version: &str) -> Result<String, AppError> {
    let v = version.trim().trim_start_matches('v').to_string();
    if !updates::valid_version(&v) {
        return Err(AppError::BadRequest(
            "That does not look like a version number.".into(),
        ));
    }
    match updates::compare_versions(&v, updates::CURRENT) {
        Some(std::cmp::Ordering::Greater) => Ok(v),
        Some(std::cmp::Ordering::Equal) => Err(AppError::BadRequest(format!(
            "This server already runs {v}."
        ))),
        _ => Err(AppError::BadRequest(format!(
            "{v} is older than the {} this server runs. EBP never goes backwards.",
            updates::CURRENT
        ))),
    }
}

#[derive(Deserialize)]
pub struct CheckRequest {
    pub version: String,
}

/// `POST /admin/update/check` — the owner pressed "Start update".
pub async fn check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CheckRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let version = check_version(&req.version)?;
    if updates::helper_installed().is_none() {
        return Err(not_installed());
    }
    let id = updates::write_request("check", &version, None).map_err(|_| not_installed())?;
    let detail = format!("asked the update helper to check version {version}");
    state
        .db
        .call(move |c| {
            audit(c, "admin.update_checked", &detail);
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "request_id": id })))
}

#[derive(Deserialize)]
pub struct ApplyRequest {
    pub version: String,
    pub digest: String,
    pub password: String,
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/update/apply` — the owner pressed "Update now" after the check.
pub async fn apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ApplyRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let version = check_version(&req.version)?;
    if updates::helper_installed().is_none() {
        return Err(not_installed());
    }
    // Only what the helper's last check showed the owner can be installed.
    let shown = updates::helper_status();
    let matches = check_is_current(
        shown.as_ref(),
        &version,
        &req.digest,
        crate::util::now_unix(),
    );
    if !matches {
        return Err(AppError::Conflict(
            "What you are approving is not what the last check showed, or that check is more than 30 minutes old. Press Start update again to check it first.".into(),
        ));
    }
    // Password first (slow hash, outside the database lock), then the second step.
    if req.password.len() > 1024 {
        return Err(AppError::BadRequest(
            "That is not your admin password.".into(),
        ));
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    if !aa::verify_login_slow(stored, req.password).await? {
        state
            .db
            .call(|c| {
                audit(
                    c,
                    "admin.update_password_wrong",
                    "wrong password when approving an update",
                );
                Ok(())
            })
            .await?;
        return Err(AppError::BadRequest(
            "That is not your admin password.".into(),
        ));
    }
    check_second_step(&state, req.code).await?;
    let digest = req.digest.clone();
    let id =
        updates::write_request("apply", &version, Some(&digest)).map_err(|_| not_installed())?;
    let detail = format!("approved the update to {version} ({digest})");
    state
        .db
        .call(move |c| {
            audit(c, "admin.update_approved", &detail);
            Ok(())
        })
        .await?;
    Ok(Json(json!({ "request_id": id })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(version: &str, digest: &str, at: i64) -> updates::HelperStatus {
        updates::HelperStatus {
            state: "ready".into(),
            code: "ok".into(),
            version: version.into(),
            digest: digest.into(),
            at,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_fresh_ready_check_for_the_same_version_and_digest_counts() {
        let d = format!("sha256:{}", "a".repeat(64));
        let other = format!("sha256:{}", "b".repeat(64));
        let now = 1_000_000;
        let ok = ready("0.2.2-rc1", &d, now - 60);
        assert!(check_is_current(Some(&ok), "0.2.2-rc1", &d, now));
        // No answer, a different digest or version, or something that is not "ready".
        assert!(!check_is_current(None, "0.2.2-rc1", &d, now));
        assert!(!check_is_current(Some(&ok), "0.2.2-rc1", &other, now));
        assert!(!check_is_current(Some(&ok), "0.2.3", &d, now));
        let mut refused = ok.clone();
        refused.state = "refused".into();
        assert!(!check_is_current(Some(&refused), "0.2.2-rc1", &d, now));
        // A check from yesterday, or one with no time, is too old to approve.
        let old = ready("0.2.2-rc1", &d, now - CHECK_MAX_AGE_SECS - 1);
        assert!(!check_is_current(Some(&old), "0.2.2-rc1", &d, now));
        let edge = ready("0.2.2-rc1", &d, now - CHECK_MAX_AGE_SECS);
        assert!(check_is_current(Some(&edge), "0.2.2-rc1", &d, now));
        let untimed = ready("0.2.2-rc1", &d, 0);
        assert!(!check_is_current(Some(&untimed), "0.2.2-rc1", &d, now));
    }
}
