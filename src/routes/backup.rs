//! `POST /admin/backup` — download an encrypted backup (see `crate::backup`).
//!
//! Needs a signed-in session + CSRF, AND the admin password again, AND a
//! separate backup password that is never stored. Rate-limited like login
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
    pub backup_password: String,
    pub backup_password_again: String,
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

    crate::backup::validate_password(&req.backup_password)?;
    if req.backup_password != req.backup_password_again {
        return Err(AppError::BadRequest(
            "the two backup passwords don't match".into(),
        ));
    }
    if req.backup_password == req.admin_password {
        return Err(AppError::BadRequest(
            "use a different password for the backup than your admin password".into(),
        ));
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
    let db_path = state.config.database_path.clone();
    let db_entry = state
        .db
        .call(move |c| crate::backup::database_entry(c, &db_path))
        .await?;
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

    let pw = req.backup_password;
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
