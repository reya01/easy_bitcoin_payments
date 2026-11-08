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

/// Alphabet for the first-run setup code: no 0/O/1/I/L lookalikes.
const SETUP_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// A one-time first-run setup code like `K7QM-2XPD-9TWE` (12 characters from
/// a 31-symbol alphabet, about 59 bits), easy to read off a screen and type
/// on a phone. Generated per boot, never stored.
pub fn generate_setup_code() -> String {
    use rand::Rng;
    let mut rng = rand::rngs::OsRng;
    let chars: Vec<char> = (0..12)
        .map(|_| SETUP_CODE_ALPHABET[rng.gen_range(0..SETUP_CODE_ALPHABET.len())] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Canonical form for comparing typed setup codes: upper-case, separators
/// and spaces dropped, so `k7qm 2xpd-9twe` matches `K7QM-2XPD-9TWE`.
pub fn normalize_setup_code(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
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
        // Browser admin: no Authorization header -> signed-in session cookie.
        let Some(header) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        else {
            return session_store(parts, state).await;
        };

        let token = header
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or(AppError::Unauthorized)?;

        // Optional lock: shop keys only work from the addresses the owner
        // listed. A stolen key is then useless anywhere else (a hardening step
        // taken because of PHP2Shell; see shopkey.rs).
        let ip = crate::ratelimit::client_ip(
            &parts.headers,
            &parts.extensions,
            state.config.trust_proxy_headers,
        );
        // The key is checked first, so only a request that really carried a
        // valid key can leave a "refused address" hint behind (anyone on the
        // internet can send a made-up key).
        let key_hash = hash_secret(token);
        let store = state
            .db
            .call(move |conn| {
                let Some(store) = store::find_by_api_key_hash(conn, &key_hash)? else {
                    return Ok(None);
                };
                if !crate::shopkey::ip_allowed(conn, &ip)? {
                    crate::shopkey::note_blocked(conn, &ip)?;
                    return Ok(None);
                }
                Ok(Some(store))
            })
            .await?;

        store
            .map(|store| StoreAuth { store })
            .ok_or(AppError::Unauthorized)
    }
}

/// Owner-only actions (change where shop notifications go, replace the key,
/// mark payments handled, ...). Needs a signed-in admin session; an API key,
/// the thing a hacked shop would hold, is never enough. (Added after the
/// PHP2Shell bugs, which can take over a PHP shop without any fault of its
/// owner: https://www.wiz.io/blog/php2shell-php-rce.)
pub struct OwnerAuth {
    pub store: Store,
}

impl FromRequestParts<AppState> for OwnerAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let StoreAuth { store } = session_store(parts, state).await?;
        Ok(OwnerAuth { store })
    }
}

/// A signed-in admin session acts as the (single) store's owner. State-
/// changing methods must also carry the session's CSRF token.
async fn session_store(parts: &Parts, state: &AppState) -> Result<StoreAuth, AppError> {
    let token = crate::admin_auth::token_from_headers(
        &parts.headers,
        crate::admin_auth::is_secure(&state.config.public_base_url),
    )
    .ok_or(AppError::Unauthorized)?;
    let csrf = state
        .db
        .call(move |c| crate::admin_auth::lookup_session(c, &token))
        .await?
        .ok_or(AppError::Unauthorized)?;
    let safe = matches!(
        parts.method,
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    if !safe {
        let supplied = parts
            .headers
            .get(crate::admin_auth::CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ct_eq(&csrf, supplied) {
            return Err(AppError::Unauthorized);
        }
    }
    let store = state
        .db
        .call(|c| store::first(c))
        .await?
        .ok_or(AppError::Unauthorized)?;
    Ok(StoreAuth { store })
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
    fn setup_codes_are_typable_and_normalise() {
        let code = generate_setup_code();
        assert_eq!(code.len(), 14);
        assert_eq!(code.matches('-').count(), 2);
        assert!(code
            .chars()
            .filter(|c| *c != '-')
            .all(|c| "ABCDEFGHJKMNPQRSTUVWXYZ23456789".contains(c)));
        assert_ne!(code, generate_setup_code());
        let sloppy = format!(" {} ", code.to_lowercase().replace('-', " "));
        assert_eq!(normalize_setup_code(&sloppy), normalize_setup_code(&code));
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
