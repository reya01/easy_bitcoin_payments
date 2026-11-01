use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::models::Store;
use crate::routes::AppState;
use crate::store;
use crate::util::random_hex;

/// SHA-256 of a secret, hex encoded. API keys are only ever stored hashed.
pub fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time string comparison for secrets.
pub fn ct_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Generate a new store API key. Shown to the merchant exactly once.
pub fn generate_api_key() -> String {
    format!("ebp_sk_{}", random_hex(32))
}

/// Generate a new webhook signing secret. Shown to the merchant exactly once.
pub fn generate_webhook_secret() -> String {
    format!("whsec_{}", random_hex(32))
}

/// Extractor that authenticates a store from `Authorization: Bearer <api key>`.
pub struct StoreAuth {
    pub store: Store,
}

impl FromRequestParts<AppState> for StoreAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(AppError::Unauthorized)?;

        let token = header
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or(AppError::Unauthorized)?;

        let key_hash = hash_secret(token);
        let store = state
            .db
            .call(move |conn| store::find_by_api_key_hash(conn, &key_hash))
            .await?;

        store.map(|store| StoreAuth { store }).ok_or(AppError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_deterministic_and_not_reversible() {
        let key = "ebp_sk_abc123";
        let a = hash_secret(key);
        let b = hash_secret(key);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(!a.contains(key));
        assert_ne!(a, hash_secret("ebp_sk_abc124"));
    }

    #[test]
    fn constant_time_eq_behaves_like_eq() {
        assert!(ct_eq("hello", "hello"));
        assert!(!ct_eq("hello", "hellp"));
        assert!(!ct_eq("hello", "hello!"));
        assert!(!ct_eq("", "x"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn generated_keys_have_expected_shape() {
        let key = generate_api_key();
        assert!(key.starts_with("ebp_sk_"));
        assert_eq!(key.len(), "ebp_sk_".len() + 64);
        assert_ne!(key, generate_api_key());

        let secret = generate_webhook_secret();
        assert!(secret.starts_with("whsec_"));
        assert_ne!(secret, generate_webhook_secret());
    }
}