//! `POST /admin/backup` — download an encrypted backup (see `crate::backup`).
//!
//! Needs a signed-in session + CSRF and the admin password again. The file
//! is locked with the admin password (never stored). Rate-limited like login
//! (it checks a password), one at a time.

use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

use crate::admin_auth as aa;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;

static IN_PROGRESS: AtomicBool = AtomicBool::new(false);

struct Busy;
impl Drop for Busy {
    fn drop(&mut self) {
        IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

#[derive(Deserialize)]
pub struct BackupRequest {
    pub admin_password: String,
    /// Authenticator code, needed only when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

/// Biggest backup file the check accepts (the same ceiling as a backup).
pub const MAX_CHECK_BYTES: usize = crate::backup::MAX_TOTAL_BYTES + 1024 * 1024;

/// `POST /admin/backup/check` — open a backup file you downloaded earlier and
/// say what is inside (file names and sizes only), without a terminal.
///
/// Body: 4 bytes (big-endian) = length of the backup password, then the
/// password (UTF-8), then the backup file. It never changes anything and
/// never returns the contents. Needs a signed-in session + CSRF; shares the
/// password brake.
pub async fn check(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    require_session(&state, &headers).await?;
    let (password, file) = split_check_body(&body)?;
    let password = password.to_string();
    let file = file.to_vec();
    let entries = tokio::task::spawn_blocking(move || crate::backup::open(&file, &password))
        .await
        .map_err(|_| AppError::Internal("backup check failed".into()))??;
    let files: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| serde_json::json!({ "name": e.name, "bytes": e.data.len() }))
        .collect();
    Ok(Json(serde_json::json!({
        "ok": true,
        "file_count": entries.len(),
        "has_database": entries.iter().any(|e| e.name == "ebp.sqlite3"),
        "has_key": entries.iter().any(|e| e.name == "ebp.key"),
        "wallet_included": entries.iter().any(|e| e.name.starts_with("barkd/")),
        "files": files,
    })))
}

/// Split the check body into (backup password, backup file).
fn split_check_body(body: &[u8]) -> Result<(&str, &[u8]), AppError> {
    let bad = || AppError::BadRequest("that upload was not understood; try again".into());
    if body.len() < 4 {
        return Err(bad());
    }
    let n = u32::from_be_bytes(body[..4].try_into().map_err(|_| bad())?) as usize;
    if n == 0 || n > crate::backup::MAX_PASSWORD_LEN * 4 || body.len() < 4 + n {
        return Err(bad());
    }
    let password = std::str::from_utf8(&body[4..4 + n]).map_err(|_| bad())?;
    Ok((password, &body[4 + n..]))
}

pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<BackupRequest>,
) -> Result<Response, AppError> {
    require_session(&state, &headers).await?;
    if IN_PROGRESS.swap(true, Ordering::SeqCst) {
        return Err(AppError::Conflict(
            "a backup is already being prepared; wait a moment".into(),
        ));
    }
    let _busy = Busy;

    // Re-check the admin password (slow hash, off the database thread).
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let admin_pw = req.admin_password.clone();
    let ok = tokio::task::spawn_blocking(move || aa::verify_login(stored.as_deref(), &admin_pw))
        .await
        .map_err(|_| AppError::Internal("backup task failed".into()))?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(&state, req.code.clone()).await?;

    // Consistent database snapshot, plus the wallet folder in managed mode.
    let db_entry = state.db.call(|c| crate::backup::database_entry(c)).await?;
    let mut entries = vec![db_entry, crate::backup::key_entry()];
    let wallet_included = match crate::managed_barkd::get() {
        Some(m) => {
            let dir = m.datadir().to_path_buf();
            let wallet = tokio::task::spawn_blocking(move || crate::backup::wallet_entries(&dir))
                .await
                .map_err(|_| AppError::Internal("backup task failed".into()))??;
            entries.extend(wallet);
            true
        }
        None => false,
    };
    let count = entries.len();

    let pw = req.admin_password.clone();
    let sealed = tokio::task::spawn_blocking(move || crate::backup::seal(&entries, &pw))
        .await
        .map_err(|_| AppError::Internal("backup task failed".into()))??;

    let size = sealed.len();
    state
        .db
        .call(move |c| {
            if let Ok(Some(store)) = crate::store::first(c) {
                let _ = crate::audit::record(
                    c,
                    &store.id,
                    "admin.backup_downloaded",
                    Some(&format!(
                        "{count} files, {size} bytes, wallet files {}",
                        if wallet_included {
                            "included"
                        } else {
                            "not included"
                        }
                    )),
                );
            }
            Ok(())
        })
        .await?;

    let name = format!("ebp-backup-{}.ebpbackup", crate::util::now_unix());
    let mut resp = (StatusCode::OK, sealed).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-ebp-wallet-included",
        HeaderValue::from_static(if wallet_included { "true" } else { "false" }),
    );
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(pw: &str, file: &[u8]) -> Vec<u8> {
        let mut v = (pw.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(pw.as_bytes());
        v.extend_from_slice(file);
        v
    }

    #[test]
    fn check_body_splits_password_and_file() {
        let b = body("pässword with spaces", b"FILEBYTES");
        let (pw, file) = split_check_body(&b).unwrap();
        assert_eq!(pw, "pässword with spaces");
        assert_eq!(file, b"FILEBYTES");
    }

    #[test]
    fn check_body_rejects_nonsense() {
        assert!(split_check_body(&[]).is_err());
        assert!(split_check_body(&[0, 0, 0]).is_err());
        assert!(
            split_check_body(&[0, 0, 0, 0, 1]).is_err(),
            "empty password"
        );
        assert!(
            split_check_body(&[0, 0, 0, 9, 1, 2]).is_err(),
            "length beyond body"
        );
        assert!(split_check_body(&[255, 255, 255, 255, 1]).is_err());
        let mut bad_utf8 = vec![0, 0, 0, 2, 0xff, 0xfe];
        bad_utf8.extend_from_slice(b"x");
        assert!(split_check_body(&bad_utf8).is_err());
    }
}
