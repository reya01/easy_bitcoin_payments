//! `POST /admin/backup` — download an encrypted backup (see `crate::backup`).
//!
//! Needs a signed-in session + CSRF and the admin password again. The file
//! is locked with the admin password unless the merchant chooses a separate
//! backup password; neither is ever stored. Rate-limited like login
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
    /// Optional. Left out, the backup is locked with the admin password.
    #[serde(default)]
    pub backup_password: Option<String>,
    #[serde(default)]
    pub backup_password_again: Option<String>,
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

    // One password by default: the admin password (already length-checked
    // when it was set). A separate one is optional and must be typed twice.
    let separate = req.backup_password.clone().filter(|p| !p.is_empty());
    if let Some(p) = &separate {
        crate::backup::validate_password(p)?;
        if req.backup_password_again.as_deref() != Some(p.as_str()) {
            return Err(AppError::BadRequest(
                "the two backup passwords don't match".into(),
            ));
        }
    }

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

    // Consistent database snapshot, plus the wallet folder in managed mode.
    let db_entry = state.db.call(|c| crate::backup::database_entry(c)).await?;
    let mut entries = vec![db_entry];
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

    let pw = separate.unwrap_or_else(|| req.admin_password.clone());
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
