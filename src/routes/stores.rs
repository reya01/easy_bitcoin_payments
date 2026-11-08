//! Store self-service: webhook configuration and API-key rotation.
//!
//! Closes review finding R4: a store created by the setup wizard had no way
//! to receive webhooks, and keys could not be rotated. Both endpoints need the
//! signed-in admin session (NOT the shop's API key, which a hacked shop would
//! hold); secrets are returned once.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::auth::{OwnerAuth, StoreAuth};
use crate::error::AppError;
use crate::routes::AppState;
use crate::store;
use crate::webhook::{allow_private_from_env, validate_webhook_url};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetWebhookRequest {
    /// `null` (or absent) disables webhooks for the store.
    pub url: Option<String>,
    /// Issue a new signing secret even if one exists.
    #[serde(default)]
    pub rotate_secret: bool,
}

/// `PUT /api/v1/stores/webhook` — set/clear the webhook URL and (re)issue
/// the signing secret. The secret appears in the response only when newly
/// generated; store it, it cannot be read back.
pub async fn set_webhook(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Json(req): Json<SetWebhookRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let url = req
        .url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    if let Some(url) = &url {
        if url.len() > 2048 {
            return Err(AppError::BadRequest("webhook url is too long".into()));
        }
        validate_webhook_url(url, allow_private_from_env()).map_err(AppError::BadRequest)?;
    }
    let store_id = auth.store.id.clone();
    let rotate = req.rotate_secret;
    let url_for_db = url.clone();
    let secret = state
        .db
        .call(move |conn| {
            let secret = store::set_webhook(conn, &store_id, url_for_db.as_deref(), rotate)?;
            // Record WHAT changed, never the secret or the full URL query.
            let detail = match (&url_for_db, &secret) {
                (None, _) => "webhook disabled".to_string(),
                (Some(_), Some(_)) => "webhook url set; signing secret issued".to_string(),
                (Some(_), None) => "webhook url set".to_string(),
            };
            crate::audit::record(conn, &store_id, "store.webhook_config", Some(&detail))?;
            Ok(secret)
        })
        .await?;
    Ok(Json(serde_json::json!({
        "webhook_url": url,
        "webhook_secret": secret,
    })))
}

/// `GET /api/v1/admin/version` — current version, whether a newer release
/// exists, and how many orders are open (the best time to update is when
/// there are none). Read-only; EBP never updates itself.
pub async fn version(
    State(state): State<AppState>,
    _auth: StoreAuth,
) -> Result<Json<serde_json::Value>, AppError> {
    let open_orders: i64 = state
        .db
        .call(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM invoices WHERE status IN ('pending', 'underpaid')",
                [],
                |r| r.get(0),
            )?)
        })
        .await?;
    let latest = if state.config.update_check {
        crate::updates::latest().await
    } else {
        None
    };
    let update_available = latest
        .as_ref()
        .is_some_and(|l| crate::updates::is_newer(&l.version, crate::updates::CURRENT));
    Ok(Json(serde_json::json!({
        "current": crate::updates::CURRENT,
        "check_enabled": state.config.update_check,
        "latest": latest,
        "update_available": update_available,
        "open_orders": open_orders,
        "host": crate::updates::host_kind(),
    })))
}

/// `POST /api/v1/stores/webhook/test` — queue a harmless `webhook.test`
/// notification to the store's webhook so the merchant can confirm their
/// shop receives and verifies it. No invoice and no payment is involved.
pub async fn test_webhook(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<serde_json::Value>, AppError> {
    if auth.store.webhook_url.is_none() {
        return Err(AppError::BadRequest(
            "set your shop's webhook address first".into(),
        ));
    }
    let store_id = auth.store.id.clone();
    state
        .db
        .call(move |conn| {
            crate::webhook_outbox::enqueue(
                conn,
                &store_id,
                "test",
                "webhook.test",
                &format!("webhook.test:{}", crate::util::random_hex(8)),
                &serde_json::json!({
                    "test": true,
                    "message": "Test notification from your payments server. No payment happened."
                }),
            )?;
            crate::audit::record(conn, &store_id, "store.webhook_test", None)?;
            Ok(())
        })
        .await?;
    Ok(Json(serde_json::json!({ "queued": true })))
}

/// `POST /api/v1/stores/rotate-key` — revoke every active API key of this
/// store and issue a new one. The calling key stops working immediately;
/// the response carries the replacement (shown once).
pub async fn rotate_key(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<serde_json::Value>, AppError> {
    let store_id = auth.store.id.clone();
    let key = state
        .db
        .call(move |conn| {
            let key = store::rotate_api_key(conn, &store_id)?;
            crate::audit::record(conn, &store_id, "store.api_keys_rotated", None)?;
            Ok(key)
        })
        .await?;
    Ok(Json(serde_json::json!({ "api_key": key })))
}
