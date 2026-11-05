//! Store self-service: webhook configuration and API-key rotation.
//!
//! Closes review finding R4: a store created by the setup wizard had no way
//! to receive webhooks, and keys could not be rotated. Both endpoints
//! authenticate with the store's API key; secrets are returned once.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::auth::StoreAuth;
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
    auth: StoreAuth,
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

/// `POST /api/v1/stores/rotate-key` — revoke every active API key of this
/// store and issue a new one. The calling key stops working immediately;
/// the response carries the replacement (shown once).
pub async fn rotate_key(
    State(state): State<AppState>,
    auth: StoreAuth,
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
