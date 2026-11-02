use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Default tolerance for the `t=` timestamp in a signature header.
pub const DEFAULT_TOLERANCE_SECS: i64 = 300;

/// Build the `X-EBP-Signature` header value:
/// `t=<unix>, v1=<hex HMAC_SHA256(secret, "<t>.<body>")>`
pub fn sign_payload(secret: &str, timestamp: i64, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(body);
    let signature = hex::encode(mac.finalize().into_bytes());
    format!("t={timestamp},v1={signature}")
}

/// Verify a signature header against a body.
///
/// Rejects malformed headers, signatures that do not match, and timestamps
/// outside `tolerance_secs` of `now` (replay protection).
pub fn verify_signature(
    secret: &str,
    header: &str,
    body: &[u8],
    now: i64,
    tolerance_secs: i64,
) -> bool {
    let mut timestamp: Option<i64> = None;
    let mut signature: Option<&str> = None;

    for part in header.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("v1=") {
            signature = Some(value);
        }
    }

    let (Some(timestamp), Some(signature)) = (timestamp, signature) else {
        return false;
    };

    if (now - timestamp).abs() > tolerance_secs {
        return false;
    }

    let expected = sign_payload(secret, timestamp, body);
    let expected_signature = expected
        .split_once("v1=")
        .map(|(_, value)| value)
        .unwrap_or_default();

    crate::auth::ct_eq(expected_signature, signature)
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("webhook request failed: {0}")]
    Transport(String),
    #[error("webhook endpoint returned HTTP {0}")]
    Status(u16),
}

/// Delivers signed webhook events to a store.
///
/// The emitter performs a single delivery attempt with a timeout. Retries
/// with backoff (1m / 5m / 30m / 2h / 12h), the delivery log, and one-tap
/// re-send are future work and require the `webhook_deliveries` table.
pub struct WebhookEmitter {
    client: reqwest::Client,
}

impl WebhookEmitter {
    pub fn new(timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .user_agent(concat!("ebp/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }

    pub async fn emit(
        &self,
        url: &str,
        secret: &str,
        event: &str,
        idempotency_key: &str,
        payload: &serde_json::Value,
    ) -> Result<(), WebhookError> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| WebhookError::Transport(format!("could not serialize payload: {e}")))?;
        let timestamp = crate::util::now_unix();
        let signature = sign_payload(secret, timestamp, &body);

        let response = self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header("x-ebp-event", event)
            .header("x-ebp-idempotency-key", idempotency_key)
            .header("x-ebp-signature", signature)
            .body(body)
            .send()
            .await
            .map_err(|e| WebhookError::Transport(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(WebhookError::Status(response.status().as_u16()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test_secret";

    #[test]
    fn signature_has_expected_shape() {
        let header = sign_payload(SECRET, 1_700_000_000, b"{\"a\":1}");
        assert!(header.starts_with("t=1700000000,v1="));
        let (_, sig) = header.split_once("v1=").unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn signature_is_deterministic() {
        let a = sign_payload(SECRET, 1_700_000_000, b"body");
        let b = sign_payload(SECRET, 1_700_000_000, b"body");
        assert_eq!(a, b);
    }

    #[test]
    fn verify_accepts_fresh_valid_signature() {
        let now = 1_700_000_000;
        let body = b"{\"event\":\"invoice.paid\"}";
        let header = sign_payload(SECRET, now, body);
        assert!(verify_signature(SECRET, &header, body, now, 300));
        assert!(verify_signature(SECRET, &header, body, now + 299, 300));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let now = 1_700_000_000;
        let header = sign_payload(SECRET, now, b"original");
        assert!(!verify_signature(SECRET, &header, b"tampered", now, 300));
    }

    #[test]
    fn verify_rejects_wrong_secret() {
        let now = 1_700_000_000;
        let body = b"payload";
        let header = sign_payload(SECRET, now, body);
        assert!(!verify_signature("whsec_other", &header, body, now, 300));
    }

    #[test]
    fn verify_rejects_stale_timestamp() {
        let now = 1_700_000_000;
        let body = b"payload";
        let header = sign_payload(SECRET, now, body);
        assert!(!verify_signature(SECRET, &header, body, now + 301, 300));
        assert!(!verify_signature(SECRET, &header, body, now - 301, 300));
    }

    #[test]
    fn verify_rejects_malformed_headers() {
        let now = 1_700_000_000;
        assert!(!verify_signature(SECRET, "", b"x", now, 300));
        assert!(!verify_signature(SECRET, "v1=abc", b"x", now, 300));
        assert!(!verify_signature(
            SECRET,
            "t=notanumber,v1=abc",
            b"x",
            now,
            300
        ));
        assert!(!verify_signature(SECRET, "t=1700000000", b"x", now, 300));
    }
}
