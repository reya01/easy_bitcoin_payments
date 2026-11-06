use std::time::Duration;

use hmac::digest::KeyInit;
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

/// Addresses a webhook must never be sent to: loopback, private, link-local,
/// unspecified, carrier-grade NAT, benchmarking/protocol ranges, cloud
/// metadata, and IPv6 forms that embed an IPv4 address (mapped `::ffff:`,
/// compatible, NAT64 `64:ff9b::/96`), which are checked as the IPv4 they carry.
fn ip_is_private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || o[0] == 0 // 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 CGNAT (incl. 100.100.100.200)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15
                || o[0] >= 224 // multicast and reserved
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_private(std::net::IpAddr::V4(v4));
            }
            let seg = v6.segments();
            if seg[..6] == [0, 0, 0, 0, 0, 0] && !v6.is_unspecified() && !v6.is_loopback() {
                // IPv4-compatible ::a.b.c.d
                let v4 = std::net::Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    seg[6] as u8,
                    (seg[7] >> 8) as u8,
                    seg[7] as u8,
                );
                return ip_is_private(std::net::IpAddr::V4(v4));
            }
            if seg[0] == 0x64 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
                // NAT64 64:ff9b::/96 carries an IPv4 address.
                let v4 = std::net::Ipv4Addr::new(
                    (seg[6] >> 8) as u8,
                    seg[6] as u8,
                    (seg[7] >> 8) as u8,
                    seg[7] as u8,
                );
                return ip_is_private(std::net::IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg[0] & 0xff00) == 0xff00 // multicast
        }
    }
}

/// Validate a merchant-supplied webhook URL (SSRF guard, beta hardening).
///
/// Rejects non-http(s) schemes, embedded credentials, and hosts that are
/// loopback / private / link-local IP literals or `localhost`-style names,
/// unless `allow_private` is true (local development, and the test suite's
/// loopback mock). LIMITATION: this is a textual check — a public DNS name
/// that resolves to a private IP (DNS rebinding) is NOT caught; pair with
/// network egress rules in production. See docs/KNOWN_ISSUES.md.
pub fn validate_webhook_url(raw: &str, allow_private: bool) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("not a valid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "scheme '{other}' is not allowed (use http or https)"
            ))
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URLs with embedded credentials are not allowed".to_string());
    }
    // A trailing dot is the same host ("localhost." == "localhost").
    let host = url
        .host_str()
        .ok_or("URL has no host")?
        .to_lowercase()
        .trim_end_matches('.')
        .to_string();
    if allow_private {
        return Ok(());
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".internal") {
        return Err("localhost/internal hosts are not allowed".to_string());
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        if ip_is_private(ip) {
            return Err("private/loopback/link-local addresses are not allowed".to_string());
        }
    }
    Ok(())
}

/// Whether private/loopback webhook targets are allowed
/// (`EBP_WEBHOOK_ALLOW_PRIVATE=1`; local development only).
pub fn allow_private_from_env() -> bool {
    std::env::var("EBP_WEBHOOK_ALLOW_PRIVATE")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
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
            // SECURITY (beta hardening): never follow redirects. A merchant
            // URL that 302s to an internal address (cloud metadata,
            // barkd on 127.0.0.1) would otherwise bypass URL validation.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("ebp/", env!("CARGO_PKG_VERSION")))
            .build()
            // Builder::build only fails on TLS-backend init (system-wide
            // breakage). A fallback WITHOUT a timeout would let one hung
            // merchant endpoint stall emit() forever — panic loudly at
            // startup instead.
            .expect("reqwest client TLS backend unavailable");
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
    fn ssrf_guard_blocks_embedded_and_unusual_private_addresses() {
        for bad in [
            "http://127.0.0.1/",
            "http://localhost./",
            "http://LOCALHOST/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[::127.0.0.1]/",
            "http://[64:ff9b::7f00:1]/",
            "http://0.0.0.0/",
            "http://0.1.2.3/",
            "http://100.100.100.200/",
            "http://100.64.0.1/",
            "http://192.0.0.1/",
            "http://198.18.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.5/",
            "http://[fd00::1]/",
            "http://[ff02::1]/",
            "http://224.0.0.1/",
        ] {
            assert!(
                validate_webhook_url(bad, false).is_err(),
                "should block {bad}"
            );
        }
        for good in [
            "https://shop.example.com/hook",
            "https://203.0.113.9/hook",
            "https://[2001:db8::1]/hook",
            "http://100.63.0.1/",
            "http://198.20.0.1/",
        ] {
            assert!(
                validate_webhook_url(good, false).is_ok(),
                "should allow {good}"
            );
        }
    }

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
    fn url_validation_blocks_internal_targets() {
        assert!(validate_webhook_url("https://shop.example.com/hook", false).is_ok());
        for bad in [
            "http://127.0.0.1:3000/x",
            "http://localhost/x",
            "http://10.0.0.5/x",
            "http://192.168.1.1/x",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/x",
            "ftp://example.com/x",
            "https://user:pw@example.com/x",
            "not a url",
        ] {
            assert!(
                validate_webhook_url(bad, false).is_err(),
                "{bad} must be rejected"
            );
        }
        assert!(validate_webhook_url("http://127.0.0.1:9/x", true).is_ok());
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
