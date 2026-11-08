//! Per-client rate limiting for the PUBLIC checkout routes (`/i/*`).
//!
//! Beta hardening: the checkout page and its `/json`, `/status`, `/rails`
//! endpoints are unauthenticated and some touch the wallet (`/rails`), so an
//! anonymous client could hammer barkd. This is a small in-memory fixed-window
//! limiter (no external crate): `public_rate_limit_per_min` requests per
//! client per 60 s window, 0 disables it.
//!
//! Client identity: the TCP peer address (needs
//! `into_make_service_with_connect_info`, wired in main.rs). Behind a reverse
//! proxy every request shares the proxy's address, so set
//! `trust_proxy_headers = true` ONLY when a proxy you control overwrites
//! `X-Forwarded-For`; otherwise clients could spoof it to dodge the limit.
//!
//! Limits: state is per-process and resets on restart; it is a speed bump
//! against casual abuse, not a DDoS defence.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::util::now_unix;

const WINDOW_SECS: i64 = 60;
/// Prune stale windows when the table grows past this many clients.
const PRUNE_ABOVE: usize = 10_000;

pub struct RateLimiter {
    per_minute: u32,
    trust_proxy_headers: bool,
    /// One shared bucket for every client (a brake that does not depend on
    /// trusting any client address).
    global: bool,
    hits: Mutex<HashMap<String, (i64, u32)>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32, trust_proxy_headers: bool) -> Self {
        Self {
            per_minute,
            trust_proxy_headers,
            global: false,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// A limiter with a single shared bucket: `per_minute` requests in total,
    /// whoever sends them.
    pub fn new_global(per_minute: u32) -> Self {
        Self {
            global: true,
            ..Self::new(per_minute, false)
        }
    }

    /// Record one request from `key` at `now`; false means "over the limit".
    pub fn allow(&self, key: &str, now: i64) -> bool {
        if self.per_minute == 0 {
            return true;
        }
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if hits.len() > PRUNE_ABOVE {
            hits.retain(|_, (start, _)| now - *start < WINDOW_SECS);
        }
        // Still too many live keys (an attacker rotating fake client
        // addresses): share one overflow bucket instead of growing the map.
        let key = if hits.len() > PRUNE_ABOVE && !hits.contains_key(key) {
            "overflow"
        } else {
            key
        };
        let entry = hits.entry(key.to_string()).or_insert((now, 0));
        if now - entry.0 >= WINDOW_SECS {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= self.per_minute
    }

    fn client_key(&self, req: &Request) -> String {
        if self.global {
            return "global".to_string();
        }
        client_ip(req.headers(), req.extensions(), self.trust_proxy_headers)
    }
}

/// The address a request came from. With `trust_proxy_headers` the RIGHTMOST
/// `X-Forwarded-For` entry is used: it is the one our own proxy appended (the
/// address it actually saw). The leftmost is whatever the client claimed, so
/// trusting it would let an attacker pick a fresh identity per request and
/// dodge the limit.
pub fn client_ip(
    headers: &axum::http::HeaderMap,
    extensions: &axum::http::Extensions,
    trust_proxy_headers: bool,
) -> String {
    if trust_proxy_headers {
        if let Some(last) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return last.to_string();
        }
    }
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// axum middleware: 429 + `Retry-After` when the client is over the limit.
pub async fn limit(State(limiter): State<Arc<RateLimiter>>, req: Request, next: Next) -> Response {
    let key = limiter.client_key(&req);
    if limiter.allow(&key, now_unix()) {
        next.run(req).await
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            "rate limit exceeded",
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_blocks_until_the_window_rolls() {
        let l = RateLimiter::new(3, false);
        assert!(l.allow("a", 1000));
        assert!(l.allow("a", 1001));
        assert!(l.allow("a", 1002));
        assert!(!l.allow("a", 1003), "4th request in the window is refused");
        assert!(l.allow("b", 1003), "other clients are unaffected");
        assert!(l.allow("a", 1060), "a new window starts after 60 s");
    }

    #[test]
    fn proxy_mode_keys_on_the_rightmost_forwarded_address() {
        let l = RateLimiter::new(3, true);
        let req = axum::http::Request::builder()
            .header("x-forwarded-for", "6.6.6.6, 203.0.113.9")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            l.client_key(&req),
            "203.0.113.9",
            "client-supplied entries are ignored"
        );
        let spoofed = axum::http::Request::builder()
            .header("x-forwarded-for", "1.2.3.4, 203.0.113.9")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            l.client_key(&spoofed),
            l.client_key(&req),
            "same real client, same bucket"
        );
    }

    #[test]
    fn global_limiter_is_one_shared_bucket() {
        let l = RateLimiter::new_global(2);
        let req = axum::http::Request::builder()
            .body(axum::body::Body::empty())
            .unwrap();
        let k = l.client_key(&req);
        assert!(l.allow(&k, 1000));
        assert!(l.allow(&k, 1001));
        assert!(!l.allow(&k, 1002));
    }

    #[test]
    fn zero_disables_the_limiter() {
        let l = RateLimiter::new(0, false);
        for i in 0..1000 {
            assert!(l.allow("a", i));
        }
    }
}
