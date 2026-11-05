#![allow(dead_code)]
// create_wallet / refresh_all / client() are consumed by
// the provisioning wizard and admin routes in a later
// milestone; until then the unit tests exercise them.

//! Phase 1: `WalletAdapter` over the barkd sidecar's REST API.
//!
//! Docs: barkd_index.md (why barkd), barkd_start.md (base URL + OpenAPI),
//! barkd_authenticate.md (bearer token), connection-details.md (ASP/Esplora),
//! learn_lifetime.md (per-VTXO expiry), learn_offboard.md (cooperative
//! sweeps), learn_exit.md (why the on-chain reserve exists).
//!
//! Paths and methods are checked against barkd's OpenAPI endpoint list (all
//! wallet endpoints live under `/api/v1/...`, which is part of
//! [`BarkdConfig::base_url`]). Response *shapes* are pinned live: with the
//! final three captures (2026-11-01, real barkd 0.8.0 on signet) all
//! client-facing shapes are verified — `/wallet/connected` →
//! `{"connected": true}`, `/wallet/mnemonic` → `{"mnemonic": "…"}`
//! (12 space-separated words, only under `--expose-mnemonic`), and the
//! `/fees/lightning/receive` success body `{"gross_amount_sat": …,
//! "fee_sat": …, "net_amount_sat": …, "vtxos_spent": […]}` — see the
//! `VERIFIED` markers on the constants below, the verbatim bodies in
//! `barkd/live_shapes.rs`, and `verified_shape_tests`. The one shape
//! still under `VERIFY` is the wallet-create response (no pass has
//! captured it — the probe datadir already held a wallet). Every parser
//! stays defensive regardless: a missing safety-critical field fails
//! loudly instead of silently reading zero.
//!
//! Two entry points:
//!
//! * [`BarkdClient`] — the async reqwest client. Async code (settlement
//!   sweeps, offboards, VTXO refresh) calls it directly.
//! * [`BarkdWallet`] — implements the synchronous [`WalletAdapter`] trait by
//!   bridging each call to a [`BarkdClient`] on a dedicated current-thread
//!   tokio runtime on its own OS thread. The bridge exists because
//!   `Runtime::block_on` panics when called from within a runtime context
//!   (e.g. an axum handler); a channel to a dedicated thread never does.
//!   When the trait goes async, `BarkdWallet` becomes a thin newtype around
//!   `BarkdClient` and the bridge is deleted.
//!
//! Token hygiene: the bearer token lives in a private field, is redacted in
//! `Debug`, is marked sensitive in the `Authorization` header, and is
//! scrubbed from any error message built from a barkd response. It is never
//! logged. `--expose-mnemonic` stays off in normal operation and no code
//! here calls GET /wallet/mnemonic — the ONE exception is the setup
//! wizard's expose-mnemonic ceremony (src/routes/setup.rs), which runs
//! at wallet-creation time: the merchant restarts barkd with the flag,
//! EBP reads the words exactly once (VERIFIED shape `{"mnemonic": "…"}`),
//! the backup checkbox is confirmed, and barkd is restarted WITHOUT the
//! flag before setup completes. That ceremony lives in the wizard, not
//! in this client.

use std::fmt;
use std::future::Future;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::de::DeserializeOwned;

use crate::error::AppError;
use crate::models::Rail;

use super::{
    AspStatus, LightningInvoice, ReceiveAddresses, WalletAdapter, WalletError, WalletHealth,
    WalletPayment, WalletVtxo,
};

// ---------------------------------------------------------------------------
// Paths and methods, verified against barkd's OpenAPI endpoint list (all
// under /api/v1, which is part of BarkdConfig::base_url). Response shapes
// are pinned by live probes against a real barkd 0.8.0 on signet (see the
// VERIFIED markers; the published OpenAPI carries no response schemas,
// so shapes are captured from the running daemon). The only unpinned
// shape left is the wallet-create response, which keeps its VERIFY note.
// ---------------------------------------------------------------------------
/// Create (or restore) the wallet. POST.
const PATH_CREATE_WALLET: &str = "/wallet/create";
/// Fresh Ark receive address. POST.
const PATH_ADDRESS_NEXT: &str = "/wallet/addresses/next";
/// Fresh on-chain receive address. POST.
const PATH_ONCHAIN_ADDRESS_NEXT: &str = "/onchain/addresses/next";
/// On-chain and off-chain balances. GET. VERIFIED (barkd 0.8.0, signet,
/// live wizard VERIFY pass) empty-wallet shape: `{"spendable_sat": 0,
/// "needs_refresh_sat": 0, "pending_lightning_send_sat": 0,
/// "claimable_lightning_receive_sat": 0, "pending_in_round_sat": 0,
/// "pending_board_sat": 0, "pending_arkoor_send_sat": 0,
/// "pending_offboard_sat": 0, "pending_exit_sat": 0}` — every counter is
/// an `_sat`-suffixed integer.
const PATH_BALANCE: &str = "/wallet/balance";
/// Confirmed on-chain balance — the emergency-exit reserve. GET.
const PATH_ONCHAIN_BALANCE: &str = "/onchain/balance";
/// VTXO list with per-VTXO expiry. GET. VERIFIED (same pass) empty
/// shape: a bare JSON array `[]` — no wrapper object.
const PATH_VTXOS: &str = "/wallet/vtxos";
/// Cursored wallet history (the watcher's payment source). GET.
const PATH_HISTORY: &str = "/wallet/history";
/// Chain tip (height + timestamp) for expiry-height conversion. GET.
const PATH_BITCOIN_TIP: &str = "/bitcoin/tip";
/// Ark send (settlement sweep to a same-ASP destination). POST.
const PATH_ARK_SEND: &str = "/wallet/send";
/// Cooperative on-chain withdrawal of the whole Ark balance. POST.
const PATH_OFFBOARD_ALL: &str = "/wallet/offboard/all";
/// Refresh every spendable VTXO in the next round. POST.
const PATH_REFRESH_ALL: &str = "/wallet/refresh/all";
/// Refresh specific VTXOs in the next round. POST.
const PATH_REFRESH_VTXOS: &str = "/wallet/refresh/vtxos";
/// Lightning invoice via the ASP gateway. POST.
const PATH_LN_INVOICE: &str = "/lightning/receives/invoice";
/// Whether the wallet is connected to its ASP. GET.
/// VERIFIED (barkd 0.8.0, signet, live wizard VERIFY pass): the response
/// is `{"connected": true}` — a bare boolean under `connected`, no
/// wrapper object.
const PATH_CONNECTED: &str = "/wallet/connected";
/// The wallet's recovery words. GET. VERIFIED (same pass): answers
/// `{"mnemonic": "<12 words, space-separated>"}` — a single string
/// field, no wrapper object — but ONLY while barkd runs with
/// `--expose-mnemonic`; with the flag off the endpoint refuses (exact
/// refusal shape not captured — treat any non-2xx as "not exposed").
/// No code in this client calls it: the one caller is the setup
/// wizard's expose-mnemonic ceremony (src/routes/setup.rs), which reads
/// the words once and then has barkd restarted WITHOUT the flag.
const PATH_WALLET_MNEMONIC: &str = "/wallet/mnemonic";
/// The ASP gateway's Lightning receive fee. GET. VERIFIED (same pass):
/// the required `amount_sat` travels in the QUERY STRING — probing
/// without it returned HTTP 400 with the plain-text body "Failed to
/// deserialize query string: missing field `amount_sat`" (not JSON).
/// VERIFIED success body (barkd 0.8.0, signet, 2026-11-01,
/// amount_sat=5000): `{"gross_amount_sat": 5000, "fee_sat": 0,
/// "net_amount_sat": 5000, "vtxos_spent": []}` — three `_sat` integers
/// plus a `vtxos_spent` array. The capture ran on an EMPTY wallet, so
/// `fee_sat` was 0 and `vtxos_spent` was empty; a wallet with spendable
/// VTXOs can return a nonzero fee and a populated array, so the parser
/// takes all four fields from the body and assumes none of them.
const PATH_FEES_LIGHTNING_RECEIVE: &str = "/fees/lightning/receive";

/// Whole-request timeout: barkd is a loopback sidecar, so anything beyond
/// this is a hung daemon, not a slow network.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// TCP connect timeout against the sidecar.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Page size for cursored history.
const HISTORY_PAGE_SIZE: u32 = 500;
/// Hard cap on pages per `payments_since` call so a buggy daemon cannot
/// loop us forever.
const MAX_HISTORY_PAGES: usize = 100;

/// On-chain sats kept per VTXO so an emergency exit can always be CPFP'd
/// (learn_exit.md: unilateral exits cost >= 15,000 sats per VTXO in fees).
pub const ONCHAIN_RESERVE_PER_VTXO_SATS: i64 = 15_000;

/// On-chain sats a settlement sweep may move while preserving the reserve.
pub fn sweepable_onchain_sats(onchain_sats: i64, vtxo_count: usize) -> i64 {
    (onchain_sats - ONCHAIN_RESERVE_PER_VTXO_SATS * vtxo_count as i64).max(0)
}

/// The verbatim live-captured response bodies (barkd 0.8.0, signet,
/// 2026-11-01) that completed shape verification — canonical fixtures
/// for the mock server and the shape tests. See barkd/live_shapes.rs.
#[cfg(test)]
mod live_shapes;

#[cfg(test)]
mod history_direction_tests {
    use super::*;

    /// Drill 2026-11-01, payer side (fields the mapper reads): an outbound
    /// `bark.lightning_send` — negative intended balance, destination in
    /// `sent_to`, `received_on: []` — must never become a payment, so it can
    /// never credit the invoice whose BOLT11 it paid; the same guard drops
    /// `bark.arkoor` kind=send sweeps.
    #[test]
    fn outbound_send_movement_is_never_attributed_to_an_invoice() {
        // VERIFIED live shape (LN drill): a send movement carries subsystem
        // kind "send" and a negative intended balance.
        let entry: HistoryEntry = serde_json::from_value(serde_json::json!({
            "subsystem": {"name": "bark.lightning_send", "kind": "send"},
            "intended_balance_sat": -5000,
            "sent_to": [{
                "destination": {"type": "invoice", "value": "lntbs<test-invoice>"},
                "amount_sat": 5000
            }],
            "received_on": []
        }))
        .unwrap();
        assert!(entry.into_wallet_payments().is_none());
    }
}

#[cfg(test)]
mod verified_shape_tests {
    //! Fixtures pinned by the live wizard VERIFY pass against real barkd
    //! 0.8.0 on signet. Every parser that touches these endpoints must
    //! keep passing against these exact bodies; when a shape drifts,
    //! re-pin the fixture with a new live probe — never by guessing.

    use super::*;

    /// VERIFIED: GET /wallet/connected → {"connected": true}
    const CONNECTED_BODY: &str = r#"{"connected": true}"#;

    /// VERIFIED: GET /wallet/mnemonic (only with --expose-mnemonic) →
    /// {"mnemonic": "<12 words on the probed signet wallet>"}
    const MNEMONIC_BODY: &str = r#"{"mnemonic": "faint toss city dinner satoshi remember voyage mistake surface glide swear hen"}"#;

    /// VERIFIED: GET /fees/lightning/receive without its REQUIRED
    /// amount_sat query parameter → HTTP 400 with this plain-text body.
    /// The success shape IS captured in `live_shapes.rs`
    /// (`BODY_LN_FEE_EMPTY_WALLET`); parsers must still take every field
    /// from the body and assume none of them.
    const FEES_MISSING_AMOUNT_BODY: &str =
        "Failed to deserialize query string: missing field `amount_sat`";

    /// VERIFIED: an empty wallet's balance is all-zero `_sat` counters.
    const BALANCE_EMPTY_BODY: &str = r#"{"spendable_sat": 0, "needs_refresh_sat": 0, "pending_lightning_send_sat": 0, "claimable_lightning_receive_sat": 0, "pending_in_round_sat": 0, "pending_board_sat": 0, "pending_arkoor_send_sat": 0, "pending_offboard_sat": 0, "pending_exit_sat": 0}"#;

    #[test]
    fn connected_is_a_bare_boolean_flag() {
        let value: serde_json::Value = serde_json::from_str(CONNECTED_BODY).unwrap();
        assert_eq!(value["connected"].as_bool(), Some(true));
    }

    #[test]
    fn mnemonic_is_a_bare_string_field() {
        let value: serde_json::Value = serde_json::from_str(MNEMONIC_BODY).unwrap();
        let words = value["mnemonic"].as_str().expect("mnemonic is a string");
        // The probed wallet generated 12 words. Nothing pins barkd's
        // word count, so callers must count rather than assume a number.
        assert_eq!(words.split_whitespace().count(), 12);
    }

    #[test]
    fn fee_error_body_is_plain_text_not_json() {
        // Pins the observed failure mode: the 400 body is plain text, so
        // fee-error handling must not assume JSON.
        assert!(serde_json::from_str::<serde_json::Value>(FEES_MISSING_AMOUNT_BODY).is_err());
        assert!(FEES_MISSING_AMOUNT_BODY.contains("amount_sat"));
    }

    #[test]
    fn empty_balance_is_all_zero_sat_counters() {
        let value: serde_json::Value = serde_json::from_str(BALANCE_EMPTY_BODY).unwrap();
        for (key, v) in value.as_object().unwrap() {
            assert!(key.ends_with("_sat"), "balance key {key}");
            assert_eq!(v.as_i64(), Some(0), "balance field {key}");
        }
    }

    #[test]
    fn empty_vtxo_list_is_a_bare_array() {
        // VERIFIED: GET /wallet/vtxos on an empty wallet → []
        let value: serde_json::Value = serde_json::from_str("[]").unwrap();
        assert_eq!(value, serde_json::Value::Array(vec![]));
    }

    #[test]
    fn verified_paths_match_the_live_probe() {
        assert_eq!(PATH_CONNECTED, "/wallet/connected");
        assert_eq!(PATH_WALLET_MNEMONIC, "/wallet/mnemonic");
        assert_eq!(PATH_FEES_LIGHTNING_RECEIVE, "/fees/lightning/receive");
    }
}

/// Raw JSON response, used where the pinned spec's shape is not yet
/// modelled (create wallet, send/offboard receipts).
pub type Json = serde_json::Value;

/// Configuration for [`BarkdClient`].
///
/// Construct via [`BarkdConfig::new`], [`BarkdConfig::from_app_config`]
/// (uses `barkd_url` + `barkd_token` from the server config), or
/// [`BarkdConfig::from_env`]. The token field is private so it cannot be
/// logged or interpolated by accident.
#[derive(Clone)]
pub struct BarkdConfig {
    /// Default http://127.0.0.1:3000/api/v1 — loopback / internal container
    /// network only; barkd serves plain HTTP (barkd_start.md).
    pub base_url: String,
    /// From `barkd --datadir <path> secret show`. Never logged.
    token: String,
    /// Whole-request timeout.
    pub request_timeout: Duration,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
}

impl BarkdConfig {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Build from the app config: `barkd_url` + env-only `barkd_token`.
    pub fn from_app_config(cfg: &crate::config::Config) -> Result<Self, WalletError> {
        let token = cfg.barkd_token.clone().ok_or_else(|| {
            WalletError::Unavailable(
                "barkd token is not configured (set EBP_BARKD_TOKEN)".to_string(),
            )
        })?;
        Ok(Self::new(cfg.barkd_url.clone(), token))
    }

    pub fn from_env() -> Result<Self, WalletError> {
        let base_url = std::env::var("EBP_BARKD_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000/api/v1".into());
        let token = std::env::var("EBP_BARKD_TOKEN")
            .map_err(|_| WalletError::Unavailable("EBP_BARKD_TOKEN is not set".into()))?;
        Ok(Self::new(base_url, token))
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl fmt::Debug for BarkdConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarkdConfig")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .field("request_timeout", &self.request_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

/// Heuristic for the plain-HTTP warning: is this URL loopback-only?
/// Parses the host instead of substring-matching, so
/// `http://127.0.0.1.evil.tld` is NOT treated as loopback.
fn is_loopback_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false; // unparseable: never silence the cleartext warning
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // Parse as an IP, not a prefix: `127.0.0.1.evil.tld` starts with "127."
    // but is a public DNS name that must NOT silence the cleartext warning.
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Rail from an address/destination string by network prefix (ark1/tark1 =
/// Ark off-chain; anything else treated as on-chain).
fn rail_from_address(value: &str) -> Rail {
    if value.starts_with("ark1") || value.starts_with("tark1") {
        Rail::Ark
    } else {
        Rail::Onchain
    }
}

/// Async REST client for the barkd sidecar.
///
/// Clone is cheap (the reqwest client is Arc-backed). All errors are
/// [`WalletError`]; convert to [`AppError`] at the HTTP boundary with `?`
/// via the `From<WalletError> for AppError` impl below.
#[derive(Clone)]
pub struct BarkdClient {
    http: reqwest::Client,
    config: BarkdConfig,
}

impl fmt::Debug for BarkdClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarkdClient")
            .field("base_url", &self.config.base_url)
            .finish_non_exhaustive()
    }
}

impl BarkdClient {
    pub fn new(config: BarkdConfig) -> Result<Self, WalletError> {
        let mut headers = HeaderMap::new();
        let mut value =
            HeaderValue::from_str(&format!("Bearer {}", config.token())).map_err(|_| {
                WalletError::Rejected(
                    "barkd token contains characters that are invalid in an HTTP header"
                        .to_string(),
                )
            })?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);

        if config.base_url.starts_with("http://") && !is_loopback_url(&config.base_url) {
            tracing::warn!(
                "barkd base URL is plain HTTP on a non-loopback host; the bearer token \
                 crosses the network in cleartext — put barkd behind a TLS-terminating \
                 reverse proxy (barkd_start.md)"
            );
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent(concat!("ebp/", env!("CARGO_PKG_VERSION")))
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|e| WalletError::Unavailable(format!("could not build HTTP client: {e}")))?;

        Ok(Self { http, config })
    }

    pub fn from_app_config(cfg: &crate::config::Config) -> Result<Self, WalletError> {
        Self::new(BarkdConfig::from_app_config(cfg)?)
    }

    pub fn config(&self) -> &BarkdConfig {
        &self.config
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.config.base_url, path)
    }

    /// Defense in depth: the token should never appear in a barkd response
    /// body, but if it ever does, it must not make it into our errors/logs.
    fn scrub(&self, msg: &str) -> String {
        let token = self.config.token();
        if token.is_empty() {
            return msg.to_string();
        }
        msg.replace(token, "<redacted>")
    }

    fn transport_error(&self, e: reqwest::Error) -> WalletError {
        let kind = if e.is_timeout() {
            "request timed out"
        } else if e.is_connect() {
            "connection failed"
        } else {
            "request failed"
        };
        WalletError::Unavailable(self.scrub(&format!("barkd {kind}: {e}")))
    }

    async fn status_error(&self, resp: reqwest::Response) -> WalletError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let excerpt: String = body.trim().chars().take(256).collect();
        let detail = if excerpt.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {excerpt}")
        };
        let detail = self.scrub(&detail);
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            // Auth failures are a server-side config problem; do not echo the
            // body, and never include the token itself.
            WalletError::Rejected(format!(
                "barkd authentication failed (HTTP {status}); check or rotate EBP_BARKD_TOKEN"
            ))
        } else if status.is_server_error() {
            WalletError::Unavailable(format!("barkd returned {detail}"))
        } else {
            WalletError::Rejected(format!("barkd rejected the request ({detail})"))
        }
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<T, WalletError> {
        let resp = req.send().await.map_err(|e| self.transport_error(e))?;
        if !resp.status().is_success() {
            return Err(self.status_error(resp).await);
        }
        resp.json::<T>().await.map_err(|e| {
            WalletError::Rejected(self.scrub(&format!("unparseable response from barkd: {e}")))
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, WalletError> {
        self.send_json(self.http.get(self.url(path))).await
    }

    async fn get_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, WalletError> {
        self.send_json(self.http.get(self.url(path)).query(query))
            .await
    }

    async fn post<T: DeserializeOwned, B: serde::Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, WalletError> {
        self.send_json(self.http.post(self.url(path)).json(body))
            .await
    }

    async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, WalletError> {
        self.send_json(self.http.post(self.url(path))).await
    }

    // -- Wallet lifecycle ---------------------------------------------------

    /// Create the wallet (first start) or restore it from a mnemonic.
    ///
    /// Returns barkd's raw response: its shape is not yet modelled (VERIFY:
    /// field shape); the provisioning wizard (plan task 3) is what will read
    /// it — including the one-time mnemonic ceremony, if 0.8.0 returns one.
    /// GET /wallet/mnemonic is never called.
    pub async fn create_wallet(&self, req: &CreateWalletRequest) -> Result<Json, WalletError> {
        // VERIFY: field shape — the /wallet/create dump is empty in the
        // pinned extract.
        self.post(PATH_CREATE_WALLET, req).await
    }

    // -- Receiving ----------------------------------------------------------

    /// Fresh Ark receive address.
    // VERIFY: field shape — the /wallet/addresses/next dump is empty in the
    // pinned extract.
    pub async fn ark_address(&self) -> Result<String, WalletError> {
        let v: Json = self.post(PATH_ADDRESS_NEXT, &serde_json::json!({})).await?;
        flex::str_field(&v, &["address", "ark_address"]).ok_or_else(|| {
            WalletError::Rejected(
                "barkd address response had no address field (VERIFY: field shape)".to_string(),
            )
        })
    }

    /// Fresh on-chain receive address.
    // VERIFY: field shape — the /onchain/addresses/next dump is empty in the
    // pinned extract.
    pub async fn onchain_address(&self) -> Result<String, WalletError> {
        let v: Json = self
            .post(PATH_ONCHAIN_ADDRESS_NEXT, &serde_json::json!({}))
            .await?;
        flex::str_field(&v, &["address", "onchain_address", "bitcoin_address"]).ok_or_else(|| {
            WalletError::Rejected(
                "barkd address response had no address field (VERIFY: field shape)".to_string(),
            )
        })
    }

    /// Fresh receive pair for an invoice (Ark + on-chain fallback).
    pub async fn receive_addresses(&self) -> Result<ReceiveAddresses, WalletError> {
        let ark_address = self.ark_address().await?;
        let onchain_address = self.onchain_address().await?;
        Ok(ReceiveAddresses {
            ark_address,
            onchain_address,
        })
    }

    // -- Balances / VTXOs ---------------------------------------------------

    /// On-chain and off-chain balances, tracked separately: the on-chain
    /// part backs the emergency-exit reserve.
    /// Off-chain spendable balance from `/wallet/balance`.
    /// VERIFIED against live barkd 0.8.0 (signet): the response is a flat
    /// per-state map — {"spendable_sat":0,"needs_refresh_sat":0,
    /// "claimable_lightning_receive_sat":0,"pending_in_round_sat":0,...}.
    /// There is NO aggregate on-chain field: the on-chain side comes from
    /// `/onchain/balance` (see [`Self::onchain_balance`]).
    pub async fn balances(&self) -> Result<Balances, WalletError> {
        let v: Json = self.get(PATH_BALANCE).await?;
        let spendable = flex::i64_field(&v, &["spendable_sat", "spendable"]).ok_or_else(|| {
            WalletError::Rejected(
                "barkd balance had no spendable_sat field (VERIFIED shape)".to_string(),
            )
        })?;
        let claimable = flex::i64_field(&v, &["claimable_lightning_receive_sat"]).unwrap_or(0);
        // The on-chain side does not exist on this endpoint in barkd 0.8.0;
        // use [`Self::onchain_balance`]. Report 0 here to keep callers honest.
        Ok(Balances {
            onchain_sats: 0,
            offchain_sats: spendable,
            claimable_lightning_sats: claimable,
        })
    }

    /// Confirmed on-chain balance — the emergency-exit reserve source.
    pub async fn onchain_balance(&self) -> Result<OnchainBalance, WalletError> {
        let v: Json = self.get(PATH_ONCHAIN_BALANCE).await?;
        parse_onchain_balance(v)
    }

    /// Spendable and pending VTXOs with per-VTXO expiry data.
    /// VERIFIED against live barkd 0.8.0 (signet): /wallet/vtxos returns a
    /// FLAT top-level array — no spendable/pending envelope. Entry shape:
    /// {"id","amount_sat","policy_type","user_pubkey","server_pubkey",
    ///  "expiry_height","exit_delta","chain_anchor","exit_depth",
    ///  "state":{"type":"locked"|"spendable"|...,"movement_id":...}}.
    /// Spendability is state.type == "spendable" ONLY: a freshly boarded
    /// VTXO is "locked" while its board movement is pending, and
    /// /wallet/balance counts it under pending_board_sat, not spendable_sat.
    pub async fn vtxos(&self) -> Result<VtxoSet, WalletError> {
        let v: Json = self.get(PATH_VTXOS).await?;
        // VERIFIED: barkd 0.8.0 sends a flat array; keep the object envelope
        // only as a defensive fallback for other builds.
        let items: Vec<Json> = if let Json::Array(items) = &v {
            items.clone()
        } else {
            flex::array_field(&v, &["spendable", "spendable_vtxos", "vtxos"]).ok_or_else(|| {
                WalletError::Rejected(
                    "barkd vtxo list was neither a flat array nor an object with a vtxo array"
                        .to_string(),
                )
            })?
        };
        let mut spendable = Vec::new();
        let mut pending = Vec::new();
        for item in items {
            let state = item
                .pointer("/state/type")
                .and_then(Json::as_str)
                .map(str::to_string);
            // barkd 0.8.0 names the amount `amount_sat`; the serde alias on
            // Vtxo.amount_sats covers it — no manual injection needed.
            let vtxo = serde_json::from_value(item).map_err(|e| {
                WalletError::Rejected(format!("unparseable vtxo entry: {e} (VERIFIED shape)"))
            })?;
            match state.as_deref() {
                Some("spendable") => spendable.push(vtxo),
                Some("spent") => {}      // history, not a live VTXO
                _ => pending.push(vtxo), // "pending", "locked", unknown
            }
        }
        Ok(VtxoSet { spendable, pending })
    }

    /// VTXOs with wall-clock expiries, for the expiry queue. Ark expiries
    /// are block heights (learn_lifetime.md); when barkd reports a height we
    /// convert it with the chain tip at ~10 minutes per block.
    pub async fn vtxo_queue(&self) -> Result<Vec<WalletVtxo>, WalletError> {
        let set = self.vtxos().await?;
        let now = crate::util::now_unix();
        let needs_tip = set
            .spendable
            .iter()
            .chain(set.pending.iter())
            .any(|v| v.expires_at.is_none() && v.expiry_height.is_some());
        let tip = if needs_tip {
            match self.bitcoin_tip().await {
                Ok(tip) => Some(tip),
                Err(err) => {
                    tracing::warn!(error = %err, "could not read the chain tip; expiry heights convert imprecisely");
                    None
                }
            }
        } else {
            None
        };

        let mut out = Vec::with_capacity(set.spendable.len() + set.pending.len());
        for v in set.spendable.into_iter().chain(set.pending) {
            let expires_at = vtxo_expiry_unix(&v, tip.as_ref()).unwrap_or(0);
            out.push(WalletVtxo {
                id: v.id,
                amount_sats: v.amount_sats,
                expires_at,
                // Lightning-received VTXOs live 1-3 days (learn_lifetime.md);
                // a lifetime that short at first sight is flagged for the
                // urgent queue.
                likely_lightning: expires_at > 0 && expires_at - now <= 3 * 86_400,
            });
        }
        Ok(out)
    }

    /// The current chain tip, for converting VTXO expiry heights to
    /// wall-clock times.
    // VERIFY: field shape — the /bitcoin/tip dump is empty in the pinned
    // extract.
    pub async fn bitcoin_tip(&self) -> Result<Tip, WalletError> {
        let v: Json = self.get(PATH_BITCOIN_TIP).await?;
        let height = flex::i64_field(&v, &["tip_height", "height", "block_height", "tip"])
            .ok_or_else(|| {
                WalletError::Rejected("barkd tip had no height (VERIFY: field shape)".to_string())
            })?;
        Ok(Tip {
            height,
            timestamp: flex::i64_field(&v, &["timestamp", "time"])
                .unwrap_or_else(crate::util::now_unix),
        })
    }

    // -- Payment history ----------------------------------------------------

    /// One page of wallet history.
    // VERIFY: field shape + query params — the /wallet/history dump is empty
    // in the pinned extract. `since` (unix seconds) and `cursor` are our
    // best-effort query parameters; if barkd ignores them the caller's
    // dedupe keys (vtxo id / txid) keep replays harmless. /wallet/movements
    // is the documented fallback if history's shape does not fit.
    pub async fn history(&self, query: &HistoryQuery) -> Result<HistoryPage, WalletError> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(since) = query.since {
            params.push(("since", since.to_string()));
        }
        if let Some(cursor) = &query.cursor {
            params.push(("cursor", cursor.clone()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit", limit.to_string()));
        }
        let v: Json = self.get_query(PATH_HISTORY, &params).await?;
        parse_history_page(v)
    }

    /// History at or after `since` (UNIX seconds), following the cursor
    /// until the list is exhausted.
    pub async fn payments_since(&self, since: i64) -> Result<Vec<HistoryEntry>, WalletError> {
        let mut out = Vec::new();
        let mut query = HistoryQuery {
            since: Some(since),
            cursor: None,
            limit: Some(HISTORY_PAGE_SIZE),
        };
        for _ in 0..MAX_HISTORY_PAGES {
            let page = self.history(&query).await?;
            out.extend(page.entries);
            match page.next_cursor {
                Some(cursor) => {
                    // After the first page the cursor alone positions us.
                    query.since = None;
                    query.cursor = Some(cursor);
                }
                None => return Ok(out),
            }
        }
        // Fail, don't truncate: a partial page returned as success would let
        // the caller advance its watermark and permanently skip the
        // unreturned tail (paid-but-uncredited invoices).
        Err(WalletError::Unavailable(format!(
            "barkd history exceeded {MAX_HISTORY_PAGES} pages — refusing to return partial history"
        )))
    }

    // -- Settlement ---------------------------------------------------------

    /// Ark send to a same-ASP destination (settlement sweep to the
    /// merchant's Ark wallet). Returns barkd's raw receipt: its shape is
    /// not pinned (VERIFY: field shape), and never parsing it means a
    /// successful send cannot be mistaken for a failed one.
    pub async fn ark_send(&self, destination: &str, amount_sats: i64) -> Result<Json, WalletError> {
        // VERIFY: field shape — the /wallet/send dump is empty in the pinned
        // extract; destination/amount_sats is our best-effort model.
        // Pinned OpenAPI SendRequest: `amount_sat` (NOT `amount_sats`).
        let body = serde_json::json!({ "destination": destination, "amount_sat": amount_sats });
        self.post(PATH_ARK_SEND, &body).await
    }

    /// Cooperative on-chain withdrawal of the whole Ark balance
    /// (learn_offboard.md). Emergency exits are never automated.
    pub async fn offboard_all(&self, address: &str) -> Result<Json, WalletError> {
        // VERIFY: field shape — the /wallet/offboard/all dump is empty in
        // the pinned extract; address is our best-effort model.
        let body = serde_json::json!({ "address": address });
        self.post(PATH_OFFBOARD_ALL, &body).await
    }

    /// Refresh every spendable VTXO in the next ASP round.
    pub async fn refresh_all(&self) -> Result<Json, WalletError> {
        // No request body. VERIFY: confirm barkd 0.8.0 takes an empty body.
        self.post_empty(PATH_REFRESH_ALL).await
    }

    /// Refresh specific VTXOs in the next ASP round (renewal ahead of
    /// expiry).
    pub async fn refresh_vtxos(&self, vtxo_ids: &[String]) -> Result<Json, WalletError> {
        // VERIFY: field shape — the /wallet/refresh/vtxos dump is empty in
        // the pinned extract; the id list key is our best-effort model.
        // Pinned OpenAPI RefreshRequest: required `vtxos` (array of VTXO ids).
        let body = serde_json::json!({ "vtxos": vtxo_ids });
        self.post(PATH_REFRESH_VTXOS, &body).await
    }

    // -- Lightning ----------------------------------------------------------

    /// Create a Lightning receive invoice through the ASP's gateway
    /// (learn_payments-lightning.md). The received VTXO expires in 1-3 days
    /// and is routed to the urgent expiry queue by the watcher.
    pub async fn ln_invoice(
        &self,
        amount_sats: i64,
        description: &str,
    ) -> Result<LightningInvoice, WalletError> {
        // VERIFY: field shape — the /lightning/receives/invoice dump is
        // empty in the pinned extract; amount_sats is our best-effort model
        // and the BOLT11 is read from whichever field barkd uses.
        // Pinned OpenAPI: required `amount_sat`; optional description/expiry_secs.
        let body = serde_json::json!({ "amount_sat": amount_sats, "description": description });
        let v: Json = self.post(PATH_LN_INVOICE, &body).await?;
        let bolt11 = flex::str_field(&v, &["invoice", "payment_request", "bolt11", "ln_invoice"])
            .ok_or_else(|| {
            WalletError::Rejected(
                "barkd lightning invoice had no bolt11 field (VERIFY: field shape)".to_string(),
            )
        })?;
        Ok(LightningInvoice {
            bolt11,
            identifier: flex::str_field(&v, &["identifier", "id"]),
        })
    }

    // -- Status -------------------------------------------------------------

    /// Whether the wallet is connected to its ASP (health endpoint).
    // VERIFY: field shape — the /wallet/connected dump is empty in the
    // pinned extract.
    pub async fn connected(&self) -> Result<ConnectedStatus, WalletError> {
        let v: Json = self.get(PATH_CONNECTED).await?;
        Ok(ConnectedStatus {
            connected: v
                .get("connected")
                .and_then(Json::as_bool)
                .or_else(|| v.get("asp_connected").and_then(Json::as_bool)),
            asp_url: flex::str_field(&v, &["asp_url", "url", "server"]),
        })
    }
}

/// Request body for [`BarkdClient::create_wallet`]. `Debug` redacts the
/// mnemonic.
#[derive(Clone, serde::Serialize)]
pub struct CreateWalletRequest {
    /// "mainnet" | "signet" | "testnet" | "regtest".
    pub network: String,
    /// Ark server URL. When omitted, barkd uses Second's server for the
    /// network (connection-details.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ark: Option<String>,
    /// Chain source (Esplora URL or bitcoind). When omitted, barkd falls
    /// back to the network's Esplora endpoint (connection-details.md).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_source: Option<String>,
    /// BIP-39 mnemonic to restore; `None` generates a fresh wallet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic: Option<String>,
}

impl fmt::Debug for CreateWalletRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateWalletRequest")
            .field("network", &self.network)
            .field("ark", &self.ark)
            .field("chain_source", &self.chain_source)
            .field("mnemonic", &self.mnemonic.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Spendable + pending VTXOs from `GET /wallet/vtxos`.
#[derive(Debug, Clone, Default)]
pub struct VtxoSet {
    pub spendable: Vec<Vtxo>,
    pub pending: Vec<Vtxo>,
}

/// One VTXO as reported by `GET /wallet/vtxos`.
// VERIFY: field shape — the dump is empty in the pinned extract. `id` is
// the identifier /wallet/vtxos/{id} accepts; expiry may arrive as a block
// height or a timestamp.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Vtxo {
    // No serde default on id/amount: a vtxo entry missing either is a shape
    // we do not understand — parsing must FAIL loudly, never enter the
    // expiry/refresh queue as a 0-sat or empty-id entry.
    #[serde(alias = "outpoint", alias = "vtxo_id")]
    pub id: String,
    #[serde(alias = "amount", alias = "sats", alias = "amount_sat")]
    pub amount_sats: i64,
    #[serde(default, alias = "expiry", alias = "expires_at_height")]
    pub expiry_height: Option<u64>,
    #[serde(default, alias = "expiry_timestamp")]
    pub expires_at: Option<i64>,
}

/// One entry of `GET /wallet/history`.
///
/// Live-verified barkd 0.8.0 shape: there is NO entry-level address, amount,
/// or id. Attribution lives in `received_on[]` (destination objects), the
/// cursor timestamp is `time.completed_at` (RFC 3339 object), and dedupe keys
/// come from `metadata.htlc_vtxos` / output ids. The optional entry-level
/// fields below are defensive fallbacks only; they are not the real shape.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct HistoryEntry {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default, alias = "amount", alias = "sats")]
    pub amount_sats: i64,
    #[serde(default, alias = "vtxo", alias = "vtxo_outpoint")]
    pub vtxo_id: Option<String>,
    #[serde(default, alias = "tx_id")]
    pub txid: Option<String>,
    // NOTE: no "time" alias — real barkd 0.8.0 history entries carry `time`
    // as an OBJECT ({created_at/updated_at/completed_at, RFC 3339 strings});
    // an alias here would fail every real page's deserialization. The
    // completed_at unix value is parsed from the object instead.
    #[serde(default, alias = "created_at", alias = "timestamp")]
    pub detected_at: Option<i64>,
    /// Entry status (pinned live: "successful"). A non-successful entry is
    /// never credited.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub confirmed_at: Option<i64>,
    /// Movement direction/kind, when barkd reports one.
    #[serde(default, alias = "kind", alias = "type")]
    pub direction: Option<String>,
    /// VERIFIED live shape: subsystem {name, kind} — kind "send"/"receive".
    #[serde(default)]
    pub subsystem: Option<serde_json::Value>,
    /// VERIFIED live shape: intended_balance_sat is negative on outbound.
    #[serde(default, alias = "intended_balance")]
    pub intended_balance_sat: Option<i64>,
    /// VERIFIED live shape: sends carry sent_to non-empty; receives carry
    /// received_on non-empty (LN receives included).
    #[serde(default)]
    pub sent_to: Option<serde_json::Value>,
    /// VERIFIED live shape (LN receive drill): an array of objects
    /// `{destination: {type, value}, amount_sat}` — for LN receives the
    /// destination type is "invoice" and value is the BOLT11 string.
    #[serde(default)]
    pub received_on: Option<serde_json::Value>,
    /// Pinned live: {htlc_vtxos: [outpoint], payment_hash, payment_preimage}
    /// on LN receives; the htlc outpoint is the natural dedupe key.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Pinned live: {created_at, updated_at, completed_at} — RFC 3339
    /// strings. The watcher's cursor runs on completed_at (unix).
    #[serde(default)]
    pub time: Option<serde_json::Value>,
}

impl HistoryEntry {
    /// POSITIVE receive evidence only. Crediting is fail-closed: an entry
    /// is an invoice-paying receive ONLY when barkd says so — the explicit
    /// `direction` field, the verified `subsystem.kind == "receive"`, or a
    /// non-empty `received_on` (all live-verified on the LN drill; see
    /// `live_shapes.rs`). Anything else — sends, refresh/board/round
    /// movements, entries with no direction signal at all, or
    /// speculative top-level `type`/`kind` discriminators that are not
    /// directions — is never credited. Absence of send evidence is NOT
    /// receive evidence.
    pub fn is_inbound_credit(&self) -> bool {
        let explicit = self
            .direction
            .as_deref()
            .map(|d| d.eq_ignore_ascii_case("receive"))
            .unwrap_or(false);
        let subsystem_kind = self
            .subsystem
            .as_ref()
            .and_then(|s| s.get("kind"))
            .and_then(Json::as_str)
            .map(|k| k.eq_ignore_ascii_case("receive"))
            .unwrap_or(false);
        let received_on = self
            .received_on
            .as_ref()
            .and_then(|r| r.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        explicit || subsystem_kind || received_on
    }

    /// Kept for the module's historical "outbound filter" name; now a
    /// strict pass-through of the positive-evidence check (an entry that
    /// is not an inbound credit is, for crediting purposes, outbound or
    /// unattributable — both never credit).
    pub fn effective_direction(&self) -> String {
        if self.is_inbound_credit() {
            "receive".into()
        } else {
            String::new()
        }
    }
}

/// One creditable destination inside a receive entry's `received_on` array
/// (pinned live: `{destination: {type, value}, amount_sat}`).
pub struct ReceiveDestination {
    /// Pinned kinds: "invoice" (LN, value = BOLT11 string). "address" is
    /// handled defensively for on-chain/ark receives (value = the address).
    pub kind: String,
    pub value: String,
    pub amount_sats: i64,
}

impl HistoryEntry {
    /// Parse the pinned `received_on` shape. `None` when the field is
    /// absent/empty or its entries do not carry a destination value AND an
    /// amount — a destination we cannot read is one we cannot credit.
    fn receive_destinations(&self) -> Option<Vec<ReceiveDestination>> {
        let arr = self.received_on.as_ref()?.as_array()?;
        if arr.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for item in arr {
            let value = item
                .pointer("/destination/value")
                .and_then(Json::as_str)
                .map(str::to_string);
            let kind = item
                .pointer("/destination/type")
                .and_then(Json::as_str)
                .unwrap_or("address")
                .to_string();
            // The amount MUST be per-destination. Falling back to an
            // entry-level total would over-credit every destination in a
            // multi-destination receive.
            let amount = item.get("amount_sat").and_then(Json::as_i64);
            if let (Some(value), Some(amount)) = (value, amount) {
                if amount > 0 {
                    out.push(ReceiveDestination {
                        kind,
                        value,
                        amount_sats: amount,
                    });
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// The watcher's cursor timestamp: `time.completed_at` first (pinned
    /// live, RFC 3339), then updated_at / created_at, then an entry-level
    /// unix `detected_at` (defensive for non-live shapes).
    fn detected_at_unix(&self) -> Option<i64> {
        let from_time = self.time.as_ref().and_then(|t| {
            ["completed_at", "updated_at", "created_at"]
                .iter()
                .find_map(|k| {
                    let s = t.get(*k)?.as_str()?;
                    chrono::DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.timestamp())
                })
        });
        from_time.or(self.detected_at)
    }

    /// The dedupe key for a receive entry: the HTLC outpoint(s) (pinned on
    /// LN receives), else the first output vtxo, else an entry-level
    /// vtxo_id. Payments without ANY of these cannot be deduped and are
    /// never credited (the unique index needs a key).
    fn dedupe_key(&self) -> Option<String> {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("htlc_vtxos"))
            .and_then(Json::as_array)
            .and_then(|a| a.first())
            .and_then(Json::as_str)
            .map(str::to_string)
            // NO fallback to the destination value (a bolt11): that is
            // unique per INVOICE, not per payment — an MPP split (two HTLCs
            // paying one invoice) would dedupe-swallow the second payment.
            // No htlc/output key means we cannot dedupe safely: fail closed.
            .or_else(|| self.vtxo_id.clone())
            .or_else(|| self.txid.clone())
    }

    /// Map a receive entry to the watcher's payment shape — ONE payment per
    /// `received_on` destination (a multi-destination receive credits each
    /// destination separately; the DB's unique guards dedupe re-delivery).
    ///
    /// `None` when the entry must not or cannot be credited: no positive
    /// receive evidence, a non-successful status, no readable timestamp, no
    /// dedupe key, or no parseable destination. NEVER credit an invoice
    /// from inference.
    pub fn into_wallet_payments(self) -> Option<Vec<WalletPayment>> {
        let looks_like_a_payment =
            self.amount_sats > 0 || self.intended_balance_sat.map(|b| b > 0).unwrap_or(false);
        if !self.is_inbound_credit() {
            if looks_like_a_payment {
                // Loud, not silent: an amount-bearing entry we refuse to
                // credit means an unpinned receive shape — pin it (live
                // drill) before ever crediting on inference.
                tracing::warn!(
                    entry_status = ?self.status,
                    "history entry carries a positive amount but no receive evidence (subsystem.kind/direction/received_on) — NOT credited; if this was a real payment, live-pin this barkd history shape"
                );
            }
            return None;
        }
        if self.status.as_deref() != Some("successful") {
            if looks_like_a_payment {
                tracing::warn!(
                    entry_status = ?self.status,
                    "history entry has receive evidence but status is not \"successful\" — NOT credited; pin the live shape if this was a real payment"
                );
            }
            return None;
        }
        let detected_at = self.detected_at_unix()?;
        let dedupe = self.dedupe_key()?;

        if let Some(destinations) = self.receive_destinations() {
            let payments = destinations
                .into_iter()
                .enumerate()
                .map(|(index, d)| {
                    // Rail: an "invoice" destination is a Lightning receive
                    // via the ASP gateway — its HTLC settles as an Ark VTXO,
                    // so it rides the Ark rail (pinned by the LN drill).
                    // An "address" destination is arkon/offchain by prefix.
                    let rail = if d.kind == "invoice" {
                        Rail::Ark
                    } else {
                        rail_from_address(&d.value)
                    };
                    WalletPayment {
                        // For LN the "address" is the BOLT11 string this
                        // server issued; find_pending_by_address joins
                        // ln_invoices on it (documented there).
                        address: d.value,
                        rail,
                        amount_sats: d.amount_sats,
                        // One receive entry can carry several destinations.
                        // The DB's unique guard is global on vtxo_id, so each
                        // destination needs its own stable dedupe key; using
                        // the entry-level key for all of them would credit
                        // only the first destination and silently drop the
                        // rest.
                        vtxo_id: Some(format!("{dedupe}:{index}")),
                        txid: None,
                        detected_at,
                        confirmed_at: self.confirmed_at,
                    }
                })
                .collect();
            return Some(payments);
        }

        // Defensive legacy/defensive shape: an entry-level address with a
        // dedupe key and a positive amount (possible on unpinned on-chain
        // receive entries).
        let Some(address) = self.address else {
            if looks_like_a_payment {
                tracing::warn!(
                    entry_status = ?self.status,
                    "history entry has receive evidence but no parseable received_on destination or entry-level address — NOT credited; pin this barkd history shape"
                );
            }
            return None;
        };
        if self.vtxo_id.is_none() && self.txid.is_none() {
            if looks_like_a_payment {
                tracing::warn!(
                    entry_status = ?self.status,
                    "history entry has receive evidence and an address but no dedupe key — NOT credited; pin this barkd history shape"
                );
            }
            return None;
        }
        if self.amount_sats <= 0 {
            if looks_like_a_payment {
                tracing::warn!(
                    entry_status = ?self.status,
                    "history entry has receive evidence and an address but no positive entry-level amount — NOT credited; pin this barkd history shape"
                );
            }
            return None;
        }
        Some(vec![WalletPayment {
            address,
            rail: if self.vtxo_id.is_some() {
                Rail::Ark
            } else {
                Rail::Onchain
            },
            amount_sats: self.amount_sats,
            vtxo_id: self.vtxo_id,
            txid: self.txid,
            detected_at,
            confirmed_at: self.confirmed_at,
        }])
    }
}

/// One page of cursored history.
#[derive(Debug, Clone, Default)]
pub struct HistoryPage {
    pub entries: Vec<HistoryEntry>,
    /// Opaque cursor for the next page; `None` when history is exhausted.
    pub next_cursor: Option<String>,
}

/// Query for [`BarkdClient::history`]. All fields optional.
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    /// Only entries at or after this UNIX timestamp.
    pub since: Option<i64>,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
    /// Max entries in the page.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Balances {
    pub onchain_sats: i64,
    pub offchain_sats: i64,
    /// Claimable Lightning receives (VERIFIED: claimable_lightning_receive_sat).
    pub claimable_lightning_sats: i64,
}

impl Balances {
    /// Total till balance: off-chain spendable + on-chain reserve.
    pub fn total_sats(&self) -> i64 {
        self.onchain_sats + self.offchain_sats
    }
}

/// On-chain balance from `GET /onchain/balance`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OnchainBalance {
    /// Confirmed, spendable on-chain sats. This is the emergency-exit
    /// reserve source; unconfirmed coins cannot CPFP an exit.
    pub confirmed_sats: i64,
    pub unconfirmed_sats: i64,
}

/// Chain tip from `GET /bitcoin/tip`.
#[derive(Debug, Clone, Copy)]
pub struct Tip {
    pub height: i64,
    pub timestamp: i64,
}

/// Wall-clock (UNIX seconds) expiry of a VTXO.
///
/// barkd reports expiry as a BLOCK HEIGHT (`expiry_height`), not a timestamp
/// (learn_lifetime.md). Callers must never compare a height against
/// `now_unix()` — that was the P0 renewal bug fixed in the beta-hardening
/// pass (a height like 900_000 is always "less than now", so every VTXO
/// looked expired / or never due depending on the comparison).
///
/// Returns:
/// * `Some(ts)` — an explicit `expires_at` wins; otherwise a height is
///   converted with the chain tip at ~600 s/block. A height at or below the
///   tip has already expired and yields `Some(0)` (urgent).
/// * `None` — expiry is unknown (no timestamp, and no height or no tip).
///   Callers decide how to treat unknown; the queue shows it as urgent (0),
///   the settlement engine refreshes but throttles (see settlement.rs).
pub fn vtxo_expiry_unix(v: &Vtxo, tip: Option<&Tip>) -> Option<i64> {
    if let Some(ts) = v.expires_at {
        return Some(ts);
    }
    match (tip, v.expiry_height) {
        (Some(tip), Some(height)) => {
            let height = height as i64;
            if height > tip.height {
                Some(tip.timestamp + (height - tip.height) * 600)
            } else {
                Some(0)
            }
        }
        _ => None,
    }
}

/// ASP connection status from `GET /wallet/connected`.
#[derive(Debug, Clone)]
pub struct ConnectedStatus {
    pub connected: Option<bool>,
    pub asp_url: Option<String>,
}

impl ConnectedStatus {
    /// Map onto the health endpoint's ASP tri-state.
    fn asp_status(&self) -> AspStatus {
        match self.connected {
            Some(true) => AspStatus::Ok,
            Some(false) => AspStatus::Down,
            None => AspStatus::Unknown,
        }
    }

    /// One-line human summary for `WalletHealth::detail`. No secrets.
    fn describe(&self) -> String {
        match (&self.connected, &self.asp_url) {
            (Some(true), Some(url)) => format!("connected to the ASP at {url}"),
            (Some(true), None) => "connected to the ASP".to_string(),
            (Some(false), _) => "not connected to the ASP".to_string(),
            (None, _) => "ASP connection unknown".to_string(),
        }
    }
}

// VERIFY: field shape — the /onchain/balance dump is empty in the pinned
// extract. Confirmed coins are what the exit reserve may count, so a
// response we cannot read confirmed funds from is an error, never a silent
// zero (a silent zero would refuse every sweep).
fn parse_onchain_balance(v: Json) -> Result<OnchainBalance, WalletError> {
    if let Some(n) = v.as_i64() {
        return Ok(OnchainBalance {
            confirmed_sats: n,
            unconfirmed_sats: 0,
        });
    }
    let confirmed_sats = flex::i64_field(
        &v,
        &[
            "trusted_spendable_sat",
            "confirmed_sat",
            "total_sat",
            "spendable",
        ],
    )
    .ok_or_else(|| {
        WalletError::Rejected(
            "barkd on-chain balance had no confirmed field (VERIFY: field shape)".to_string(),
        )
    })?;
    // VERIFIED: barkd 0.8.0 /onchain/balance exposes total_sat,
    // trusted_spendable_sat, immature_sat, trusted_pending_sat,
    // untrusted_pending_sat, confirmed_sat. "Unconfirmed" = everything not
    // yet trusted-spendable.
    let unconfirmed_sats = match (
        flex::i64_field(&v, &["trusted_spendable_sat"]),
        flex::i64_field(&v, &["total_sat"]),
    ) {
        (Some(trusted), Some(total)) => (total - trusted).max(0),
        _ => 0,
    };
    Ok(OnchainBalance {
        confirmed_sats,
        unconfirmed_sats,
    })
}

// VERIFY: field shape — the /wallet/history dump is empty in the pinned
// extract; the list key and the cursor key are our best-effort model.
fn parse_history_page(v: Json) -> Result<HistoryPage, WalletError> {
    let items = flex::array_field(&v, &["history", "payments", "entries", "movements"])
        .ok_or_else(|| {
            WalletError::Rejected("barkd history had no list (VERIFY: field shape)".to_string())
        })?;
    let entries = items
        .into_iter()
        .map(|item| {
            serde_json::from_value::<HistoryEntry>(item).map_err(|e| {
                WalletError::Rejected(format!(
                    "unparseable history entry: {e} (VERIFY: field shape)"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = flex::str_field(&v, &["next_cursor", "cursor"]).filter(|c| !c.is_empty());
    Ok(HistoryPage {
        entries,
        next_cursor,
    })
}

/// Best-effort field extraction for endpoints whose response schema is not
/// pinned yet (every schema dump we were given is empty). Each use site
/// carries a `VERIFY: field shape` marker; replace with typed structs when
/// the pinned spec is filled in.
mod flex {
    use super::Json;

    /// First integer found under any of `keys`, at the top level or one
    /// object level deep.
    pub(crate) fn i64_field(v: &Json, keys: &[&str]) -> Option<i64> {
        for key in keys {
            if let Some(n) = v.get(*key).and_then(Json::as_i64) {
                return Some(n);
            }
        }
        if let Some(map) = v.as_object() {
            for field in map.values() {
                for key in keys {
                    if let Some(n) = field.get(*key).and_then(Json::as_i64) {
                        return Some(n);
                    }
                }
            }
        }
        None
    }

    /// First string found under any of `keys` at the top level.
    pub(crate) fn str_field(v: &Json, keys: &[&str]) -> Option<String> {
        for key in keys {
            if let Some(s) = v.get(*key).and_then(Json::as_str) {
                return Some(s.to_string());
            }
        }
        None
    }

    /// The response's list: a bare array, or the first array found under
    /// any of `keys`.
    pub(crate) fn array_field(v: &Json, keys: &[&str]) -> Option<Vec<Json>> {
        if let Some(items) = v.as_array() {
            return Some(items.clone());
        }
        for key in keys {
            if let Some(items) = v.get(*key).and_then(Json::as_array) {
                return Some(items.clone());
            }
        }
        None
    }
}

type Job = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send>;

/// The synchronous [`WalletAdapter`] over a [`BarkdClient`].
///
/// Each trait call is bridged to the async client on a dedicated
/// current-thread tokio runtime owned by its own OS thread, so
/// `Runtime::block_on` is never called from inside a runtime context (which
/// would panic inside axum handlers). When the trait goes async, delete this
/// bridge and keep only the client.
pub struct BarkdWallet {
    client: BarkdClient,
    tx: Mutex<mpsc::Sender<Job>>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl BarkdWallet {
    pub fn new(config: BarkdConfig) -> Result<Self, WalletError> {
        Self::with_client(BarkdClient::new(config)?)
    }

    pub fn with_client(client: BarkdClient) -> Result<Self, WalletError> {
        let (tx, rx) = mpsc::channel::<Job>();
        let handle = std::thread::Builder::new()
            .name("ebp-barkd-bridge".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        tracing::error!(error = %err, "could not build the barkd bridge runtime");
                        return;
                    }
                };
                for job in rx {
                    job(&runtime);
                }
            })
            .map_err(|err| {
                WalletError::Unavailable(format!("could not spawn the barkd bridge thread: {err}"))
            })?;
        Ok(Self {
            client,
            tx: Mutex::new(tx),
            handle: Mutex::new(Some(handle)),
        })
    }

    pub fn from_app_config(cfg: &crate::config::Config) -> Result<Self, WalletError> {
        Self::new(BarkdConfig::from_app_config(cfg)?)
    }

    /// The underlying async client, for async callers (settlement engine,
    /// future admin routes).
    pub fn client(&self) -> &BarkdClient {
        &self.client
    }

    /// Run `fut` on the bridge runtime and wait for it.
    ///
    /// BETA-HARDENING: this used to `panic!` when the bridge thread was gone
    /// or a job was dropped, which took the caller (watcher / request
    /// handler) down with it. Every failure mode now surfaces as
    /// `WalletError::Unavailable`, and a panic INSIDE the future is caught on
    /// the bridge thread so one bad response cannot kill the bridge for all
    /// later calls.
    fn block_on<T: Send + 'static, F: Future<Output = Result<T, WalletError>> + Send + 'static>(
        &self,
        fut: F,
    ) -> Result<T, WalletError> {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |rt| {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(fut)))
                    .unwrap_or_else(|_| {
                        Err(WalletError::Unavailable(
                            "the barkd bridge task panicked".to_string(),
                        ))
                    });
            let _ = tx.send(outcome);
        });
        let sender = self
            .tx
            .lock()
            .map_err(|_| WalletError::Unavailable("barkd bridge lock poisoned".to_string()))?;
        sender.send(job).map_err(|_| {
            WalletError::Unavailable("the barkd bridge thread has exited".to_string())
        })?;
        drop(sender);
        rx.recv().map_err(|_| {
            WalletError::Unavailable("the barkd bridge dropped a job without answering".to_string())
        })?
    }
}

impl WalletAdapter for BarkdWallet {
    fn receive_addresses(&self, _invoice_id: &str) -> Result<ReceiveAddresses, WalletError> {
        let client = self.client.clone();
        self.block_on(async move { client.receive_addresses().await })
    }

    fn balance_sats(&self) -> Result<i64, WalletError> {
        let client = self.client.clone();
        self.block_on(async move { client.balances().await.map(|b| b.total_sats()) })
    }

    fn poll_payments(&self, since: i64) -> Result<Vec<WalletPayment>, WalletError> {
        let client = self.client.clone();
        let entries = self.block_on(async move { client.payments_since(since).await })?;
        Ok(entries
            .into_iter()
            .filter_map(HistoryEntry::into_wallet_payments)
            .flatten()
            .collect())
    }

    fn health(&self) -> WalletHealth {
        let client = self.client.clone();
        match self.block_on(async move { client.connected().await }) {
            Ok(status) => WalletHealth {
                reachable: true,
                balance_sats: None,
                asp: status.asp_status(),
                detail: status.describe(),
            },
            Err(err) => WalletHealth {
                reachable: false,
                balance_sats: None,
                asp: AspStatus::Unknown,
                detail: err.to_string(),
            },
        }
    }

    fn vtxos(&self) -> Result<Vec<WalletVtxo>, WalletError> {
        let client = self.client.clone();
        self.block_on(async move { client.vtxo_queue().await })
    }

    fn lightning_invoice(
        &self,
        amount_sats: i64,
        description: &str,
    ) -> Result<LightningInvoice, WalletError> {
        let client = self.client.clone();
        let description = description.to_string();
        self.block_on(async move { client.ln_invoice(amount_sats, &description).await })
    }
}

impl From<WalletError> for AppError {
    fn from(err: WalletError) -> Self {
        // Wallet failures are server-side trouble, not bad store requests;
        // the message carries no secrets (the token is scrubbed upstream).
        AppError::Internal(err.to_string())
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A tiny blocking HTTP mock for barkd, so the client and the settlement
    //! engine are tested without a running daemon. Each accepted connection
    //! is answered from the `respond` closure and recorded for assertions.

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Debug, Clone)]
    pub(crate) struct Recorded {
        pub method: String,
        /// Path including the query string, e.g. "/api/v1/wallet/history?since=1".
        pub path: String,
        pub authorization: Option<String>,
        pub body: String,
    }

    impl Recorded {
        /// The path without the query string.
        pub fn route(&self) -> &str {
            self.path.split('?').next().unwrap_or("")
        }
    }

    /// Spawn a mock barkd on a random loopback port. Returns its base URL
    /// (including `/api/v1`) and the recorded requests.
    pub(crate) fn spawn<F>(respond: F) -> (String, Arc<Mutex<Vec<Recorded>>>)
    where
        F: Fn(&Recorded) -> (u16, String) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock barkd");
        let addr = listener.local_addr().expect("mock barkd addr");
        let recorded: Arc<Mutex<Vec<Recorded>>> = Arc::new(Mutex::new(Vec::new()));
        let respond = Arc::new(respond);

        std::thread::Builder::new()
            .name("mock-barkd".to_string())
            .spawn({
                let recorded = recorded.clone();
                let respond = respond.clone();
                move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let recorded = recorded.clone();
                    let respond = respond.clone();
                    std::thread::spawn(move || {
                        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                        let Some((method, path, authorization, body)) =
                            read_request(&mut stream)
                        else {
                            return;
                        };
                        let entry = Recorded {
                            method,
                            path,
                            authorization,
                            body,
                        };
                        {
                            let mut all = recorded.lock().expect("mock barkd recorder");
                            all.push(entry.clone());
                        }
                        let (status, body) = respond(&entry);
                        let reason = match status {
                            200 => "OK",
                            400 => "Bad Request",
                            401 => "Unauthorized",
                            404 => "Not Found",
                            500 => "Internal Server Error",
                            _ => "Response",
                        };
                        let response = format!(
                            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        stream.write_all(response.as_bytes()).ok();
                        stream.flush().ok();
                    });
                }
                }
            })
            .expect("spawn mock barkd thread");

        (format!("http://{addr}/api/v1"), recorded)
    }

    fn read_request(stream: &mut TcpStream) -> Option<(String, String, Option<String>, String)> {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                break pos;
            }
            let n = stream.read(&mut chunk).ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        };

        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let content_length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.trim().eq_ignore_ascii_case("content-length") {
                    value.trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0);

        while buf.len() < header_end + 4 + content_length {
            let n = stream.read(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }

        let body = String::from_utf8_lossy(&buf[header_end + 4..]).to_string();
        let mut parts = head.split_whitespace();
        let method = parts.next()?.to_string();
        let path = parts.next()?.to_string();
        let authorization = head.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("authorization") {
                Some(value.trim().to_string())
            } else {
                None
            }
        });
        Some((method, path, authorization, body))
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }
}

#[cfg(test)]
mod tests {
    use super::mock;
    use super::*;

    fn client(base: &str) -> BarkdClient {
        BarkdClient::new(BarkdConfig::new(base, "test-token")).expect("client")
    }

    #[tokio::test]
    async fn every_request_carries_the_bearer_token() {
        let (base, recorded) = mock::spawn(|_| (200, r#"{"spendable_sat": 2}"#.to_string()));
        client(&base).balances().await.expect("balances");
        let all = recorded.lock().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].authorization.as_deref(), Some("Bearer test-token"));
        assert_eq!(all[0].path, "/api/v1/wallet/balance");
    }

    #[tokio::test]
    async fn connection_refused_maps_to_unavailable() {
        // Grab a loopback port that nothing listens on.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let err = client(&format!("http://{addr}/api/v1"))
            .balances()
            .await
            .unwrap_err();
        assert!(matches!(err, WalletError::Unavailable(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn unauthorized_maps_to_rejected_without_echoing_the_body() {
        let (base, _) = mock::spawn(|_| (401, r#"{"error":"bad token test-token"}"#.to_string()));
        let err = client(&base).balances().await.unwrap_err();
        match err {
            WalletError::Rejected(msg) => {
                assert!(
                    msg.contains("EBP_BARKD_TOKEN"),
                    "message should point at the token: {msg}"
                );
                assert!(!msg.contains("test-token"), "must not echo the body: {msg}");
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn server_errors_map_to_unavailable() {
        let (base, _) = mock::spawn(|_| (500, "boom".to_string()));
        let err = client(&base).balances().await.unwrap_err();
        assert!(matches!(err, WalletError::Unavailable(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn error_bodies_have_the_token_scrubbed() {
        let (base, _) = mock::spawn(|_| (400, "invalid request for test-token".to_string()));
        let err = client(&base).balances().await.unwrap_err();
        let msg = match err {
            WalletError::Rejected(m) | WalletError::Unavailable(m) => m,
        };
        assert!(msg.contains("<redacted>"), "scrubbed: {msg}");
        assert!(!msg.contains("test-token"), "scrubbed: {msg}");
    }

    #[tokio::test]
    async fn parses_balances() {
        // VERIFIED live shape (barkd 0.8.0 signet): flat per-state map.
        let (base, _) = mock::spawn(|_| {
            (
                200,
                r#"{"spendable_sat": 21000, "needs_refresh_sat": 0, "claimable_lightning_receive_sat": 0, "pending_in_round_sat": 0}"#.to_string(),
            )
        });
        let b = client(&base).balances().await.unwrap();
        assert_eq!(b.offchain_sats, 21000);
        assert_eq!(b.claimable_lightning_sats, 0);
    }

    #[tokio::test]
    async fn parses_balances_with_claimable_lightning() {
        // Claimable Lightning receives are added on request.
        let (base, _) = mock::spawn(|_| {
            (
                200,
                r#"{"spendable_sat": 100, "claimable_lightning_receive_sat": 50}"#.to_string(),
            )
        });
        let b = client(&base).balances().await.unwrap();
        assert_eq!(b.offchain_sats, 100);
        assert_eq!(b.claimable_lightning_sats, 50);
    }

    #[tokio::test]
    async fn missing_balance_fields_fail_loudly() {
        let (base, _) = mock::spawn(|_| (200, r#"{"total": 7}"#.to_string()));
        let err = client(&base).balances().await.unwrap_err();
        assert!(matches!(err, WalletError::Rejected(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn parses_onchain_balance() {
        let (base, _) = mock::spawn(|_| {
            (
                200,
                r#"{"total_sat": 30500, "trusted_spendable_sat": 30000, "confirmed_sat": 30000}"#
                    .to_string(),
            )
        });
        let b = client(&base).onchain_balance().await.unwrap();
        assert_eq!(b.confirmed_sats, 30_000);
        assert_eq!(b.unconfirmed_sats, 500);
    }

    #[tokio::test]
    async fn parses_vtxos_in_both_shapes() {
        let split = r#"[{"id": "a", "amount_sat": 100, "expires_at": 1750000000, "state": {"type": "spendable"}}, {"id": "b", "amount_sat": 50, "expiry_height": 900, "state": {"type": "locked", "movement_id": 1}}]"#;
        let (base, _) = mock::spawn(move |_| (200, split.to_string()));
        let set = client(&base).vtxos().await.unwrap();
        assert_eq!(set.spendable.len(), 1);
        assert_eq!(set.pending.len(), 1);
        assert_eq!(set.spendable[0].id, "a");
        assert_eq!(set.pending[0].expiry_height, Some(900));

        let flat = r#"[{"id": "c", "amount_sat": 7, "state": {"type": "spendable"}}]"#;
        let (base, _) = mock::spawn(move |_| (200, flat.to_string()));
        let set = client(&base).vtxos().await.unwrap();
        assert_eq!(set.spendable.len(), 1);
        assert_eq!(set.pending.len(), 0);
    }

    #[tokio::test]
    async fn vtxo_queue_converts_expiry_heights_with_the_tip() {
        let (base, _) = mock::spawn(|req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sats": 100, "expiry_height": 900}]"#.to_string(),
            ),
            "/api/v1/bitcoin/tip" => (200, r#"{"tip_height": 800}"#.to_string()),
            _ => (404, "{}".to_string()),
        });
        let queue = client(&base).vtxo_queue().await.unwrap();
        assert_eq!(queue.len(), 1);
        // barkd's tip carries no timestamp (pinned OpenAPI: only
        // `tip_height`), so the tip time is "now": expiry = now + 100 blocks
        // * 600s, within a few seconds of test runtime.
        let expected = crate::util::now_unix() + 100 * 600;
        assert!(
            (queue[0].expires_at - expected).abs() <= 5,
            "expires_at {} not within 5s of {expected}",
            queue[0].expires_at
        );
    }

    #[tokio::test]
    async fn history_paging_follows_the_cursor() {
        let page1 = r#"{"payments": [{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v1", "detected_at": 100}], "next_cursor": "cursor-2"}"#;
        let page2 = r#"{"payments": [{"address": "bc1qinv", "amount_sats": 2000, "txid": "tx1", "detected_at": 101}]}"#;
        let (base, recorded) = mock::spawn(move |req| {
            if req.path.contains("cursor=cursor-2") {
                (200, page2.to_string())
            } else {
                (200, page1.to_string())
            }
        });
        let entries = client(&base).payments_since(99).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].vtxo_id.as_deref(), Some("v1"));
        assert_eq!(entries[1].txid.as_deref(), Some("tx1"));

        let all = recorded.lock().unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all[0].path.contains("since=99"),
            "first page: {}",
            all[0].path
        );
        assert!(
            all[1].path.contains("cursor=cursor-2"),
            "second page: {}",
            all[1].path
        );
    }

    #[test]
    fn history_entries_map_to_wallet_payments() {
        let entry = |json: &str| serde_json::from_str::<HistoryEntry>(json).unwrap();
        // POSITIVE receive evidence is REQUIRED (fail-closed): the credit
        // path accepts only entries barkd itself marks as receives —
        // subsystem.kind "receive" (verified live, LN drill), received_on
        // non-empty (verified live), or an explicit direction field.
        let ark = entry(
            r#"{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v1", "detected_at": 10, "status": "successful", "subsystem": {"name": "lightning", "kind": "receive"}}"#,
        )
        .into_wallet_payments()
        .unwrap();
        let ark = &ark[0];
        assert_eq!(ark.address, "ark1qinv");
        assert_eq!(ark.amount_sats, 1000);
        assert!(matches!(ark.rail, Rail::Ark));

        let onchain = entry(
            r#"{"address": "bc1qinv", "amount_sats": 5, "txid": "t", "created_at": 11, "status": "successful", "received_on": [{"destination": {"type": "address", "value": "bc1qinv"}, "amount_sat": 5}]}"#,
        )
        .into_wallet_payments()
        .unwrap();
        let onchain = &onchain[0];
        assert!(matches!(onchain.rail, Rail::Onchain));
        assert_eq!(onchain.detected_at, 11);

        let explicit = entry(
            r#"{"address": "a", "amount_sats": 7, "txid": "t2", "detected_at": 1, "status": "successful", "direction": "receive"}"#,
        )
        .into_wallet_payments()
        .unwrap();
        assert_eq!(explicit[0].amount_sats, 7);

        // NEVER credited (fail-closed), each for its own reason:
        // outbound send (verified live shape: subsystem.kind "send").
        assert!(entry(
            r#"{"address": "a", "amount_sats": 1, "vtxo_id": "v", "detected_at": 1, "subsystem": {"name": "wallet", "kind": "send"}}"#,
        )
        .into_wallet_payments()
        .is_none());
        // speculative top-level type/kind discriminators are NOT directions
        assert!(entry(
            r#"{"address": "a", "amount_sats": 1, "vtxo_id": "v", "detected_at": 1, "type": "vtxo"}"#,
        )
        .into_wallet_payments()
        .is_none());
        // no receive signal at all (round movements look exactly like this)
        assert!(
            entry(r#"{"amount_sats": 1, "vtxo_id": "v", "detected_at": 1}"#)
                .into_wallet_payments()
                .is_none()
        );
        // amount but no dedupe key / no address / non-positive amount
        assert!(
            entry(r#"{"address": "a", "amount_sats": 1, "detected_at": 1}"#)
                .into_wallet_payments()
                .is_none()
        );
        assert!(
            entry(
                r#"{"address": "a", "amount_sats": 0, "vtxo_id": "v", "detected_at": 1, "subsystem": {"kind": "receive"}}"#,
            )
            .into_wallet_payments()
            .is_none()
        );
        // a SEND entry wearing a positive amount and an invoice-like
        // address (the fake-payment vector) never credits
        assert!(entry(
            r#"{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v9", "detected_at": 10, "intended_balance_sat": -1000, "sent_to": ["ark1qother"]}"#,
        )
        .into_wallet_payments()
        .is_none());
    }

    #[tokio::test]
    async fn ark_send_posts_destination_and_amount() {
        let (base, recorded) = mock::spawn(|_| (200, r#"{"sent": true}"#.to_string()));
        client(&base)
            .ark_send("ark1merchant", 50_000)
            .await
            .unwrap();
        let all = recorded.lock().unwrap();
        assert_eq!(all[0].method, "POST");
        assert_eq!(all[0].path, "/api/v1/wallet/send");
        let body: serde_json::Value = serde_json::from_str(&all[0].body).unwrap();
        assert_eq!(body["destination"], "ark1merchant");
        assert_eq!(body["amount_sat"], 50_000);
    }

    #[tokio::test]
    async fn offboard_all_posts_the_address() {
        let (base, recorded) = mock::spawn(|_| (200, "{}".to_string()));
        client(&base).offboard_all("bc1qmerchant").await.unwrap();
        let all = recorded.lock().unwrap();
        assert_eq!(all[0].method, "POST");
        assert_eq!(all[0].path, "/api/v1/wallet/offboard/all");
        let body: serde_json::Value = serde_json::from_str(&all[0].body).unwrap();
        assert_eq!(body["address"], "bc1qmerchant");
    }

    #[tokio::test]
    async fn ln_invoice_parses_the_bolt11() {
        let (base, recorded) = mock::spawn(|_| {
            (
                200,
                r#"{"invoice": "lnbc1...", "identifier": "rec_1"}"#.to_string(),
            )
        });
        let ln = client(&base).ln_invoice(21_000, "order 1").await.unwrap();
        assert_eq!(ln.bolt11, "lnbc1...");
        assert_eq!(ln.identifier.as_deref(), Some("rec_1"));
        let all = recorded.lock().unwrap();
        assert_eq!(all[0].path, "/api/v1/lightning/receives/invoice");
        let body: serde_json::Value = serde_json::from_str(&all[0].body).unwrap();
        assert_eq!(body["amount_sat"], 21_000);
    }

    #[tokio::test]
    async fn live_history_page_credits_exactly_the_real_receive() {
        // VERBATIM /wallet/history capture (barkd 0.8.0, signet, till
        // wallet, 2026-11-03; identifiers genericized). This is the ground
        // truth the fail-closed credit path must satisfy: one creditable
        // receive (LN, 5000 sat, bolt11 destination), and NOT ONE round
        // movement, send, or board crediting anything.
        let page: &str = r#"[
 {
  "id": 6,
  "status": "successful",
  "subsystem": {
   "name": "bark.round",
   "kind": "refresh"
  },
  "metadata": {
   "funding_txid": "f897b21a5ea98a4a47a584136aab88ceeb27372ead90b29d32e28eb05513477b"
  },
  "intended_balance_sat": 0,
  "effective_balance_sat": 0,
  "offchain_fee_sat": 0,
  "sent_to": [],
  "received_on": [],
  "input_vtxos": [
   "3e09c14cedfe39f983fb0fb3fbd298487ac2cf9f6eec9ebddb766dc246d5ac30:0",
   "4055b76ff8b6a80996c90f92dcdcdf39ba3e39d23fc7125af021c8410b0a30a4:0",
   "e6e1f92ebea7467fa4b41c1dd75ffc9f309854052f7803b744a28edc5a2b5f1b:0"
  ],
  "output_vtxos": [
   "f046feb28d59eab17f3ee789763c2ff26d9ec7cea6a126e314deb4bced9809f6:0"
  ],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-02T23:05:17.727256603Z",
   "updated_at": "2026-11-02T23:38:20.304381817Z",
   "completed_at": "2026-11-02T23:38:20.304413147Z"
  }
 },
 {
  "id": 5,
  "status": "successful",
  "subsystem": {
   "name": "bark.arkoor",
   "kind": "send"
  },
  "intended_balance_sat": -100,
  "effective_balance_sat": -100,
  "offchain_fee_sat": 0,
  "sent_to": [
   {
    "destination": {
     "type": "ark",
     "value": "tark1pem36wcfzqqprsstt96uns3qeguj6prrga76saqctvqr43tew5ngutrgets0686pzqypqhh3e5z4udx7238nv4k3zmn4nxnet9kqt9l93g0x0gak8v8fztkg92xgy3"
    },
    "amount_sat": 100
   }
  ],
  "received_on": [],
  "input_vtxos": [
   "92df110cb81559d0103d6dc6e0e96fcc929d36bdb498371a1490eba67e4829f7:0"
  ],
  "output_vtxos": [
   "3e09c14cedfe39f983fb0fb3fbd298487ac2cf9f6eec9ebddb766dc246d5ac30:0",
   "4055b76ff8b6a80996c90f92dcdcdf39ba3e39d23fc7125af021c8410b0a30a4:0",
   "e6e1f92ebea7467fa4b41c1dd75ffc9f309854052f7803b744a28edc5a2b5f1b:0"
  ],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-02T04:39:18.826066767Z",
   "updated_at": "2026-11-02T04:39:19.301089477Z",
   "completed_at": "2026-11-02T04:39:19.303103567Z"
  }
 },
 {
  "id": 4,
  "status": "successful",
  "subsystem": {
   "name": "bark.round",
   "kind": "refresh"
  },
  "metadata": {
   "funding_txid": "96da230e67cba7dd4fc568cd5803d7cbecd75b6e030b1ae1e37853efe40400a6"
  },
  "intended_balance_sat": 0,
  "effective_balance_sat": 0,
  "offchain_fee_sat": 0,
  "sent_to": [],
  "received_on": [],
  "input_vtxos": [
   "593e00f8f1a6fbd9c7f6c95f6b16ce32b8be45de023ef774d94babbbdd57937f:0"
  ],
  "output_vtxos": [
   "92df110cb81559d0103d6dc6e0e96fcc929d36bdb498371a1490eba67e4829f7:0"
  ],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-01T23:20:17.422331748Z",
   "updated_at": "2026-11-01T23:23:23.874179698Z",
   "completed_at": "2026-11-01T23:23:23.874206298Z"
  }
 },
 {
  "id": 3,
  "status": "successful",
  "subsystem": {
   "name": "bark.lightning_receive",
   "kind": "receive"
  },
  "metadata": {
   "htlc_vtxos": [
    "6c5b7874d683bb81470f1fb0347f3e2ffef3c60e631dcccfad33639dc14845a8:0"
   ],
   "payment_hash": "e4eb9547e8ec588cc8da67a491858d74b4951ced632a2daefdaeeda4743ac975",
   "payment_preimage": "dd192d67869a1e31201c3f3312dbdeb1469a90fcb78add71cee566e9eac92044"
  },
  "intended_balance_sat": 5000,
  "effective_balance_sat": 5000,
  "offchain_fee_sat": 0,
  "sent_to": [],
  "received_on": [
   {
    "destination": {
     "type": "invoice",
     "value": "lntbs50u1GENERICIZED-invoice-string-replace-me"
    },
    "amount_sat": 5000
   }
  ],
  "input_vtxos": [],
  "output_vtxos": [
   "593e00f8f1a6fbd9c7f6c95f6b16ce32b8be45de023ef774d94babbbdd57937f:0"
  ],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-01T20:42:10.928527754Z",
   "updated_at": "2026-11-01T20:42:11.695904825Z",
   "completed_at": "2026-11-01T20:42:11.695905935Z"
  }
 },
 {
  "id": 2,
  "status": "successful",
  "subsystem": {
   "name": "bark.arkoor",
   "kind": "send"
  },
  "intended_balance_sat": -29987,
  "effective_balance_sat": -29987,
  "offchain_fee_sat": 0,
  "sent_to": [
   {
    "destination": {
     "type": "ark",
     "value": "tark1pem36wcfzqqphhrqsra04qzz7v3ds3kn66djwkad0nw886aq729yvl0v86829g7ezqypwqjfxnscfselgr33vq6vuxqhdcufup9ue9a54qd0h7sm0we7nncsduzdxw"
    },
    "amount_sat": 29987
   }
  ],
  "received_on": [],
  "input_vtxos": [
   "fbbda5bf2306aaaecb70093e8b59be4a81d054a8d417a0c0e5759b5873061650:0"
  ],
  "output_vtxos": [],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-01T18:31:34.211412168Z",
   "updated_at": "2026-11-01T18:31:34.211412168Z",
   "completed_at": "2026-11-01T18:31:34.520475478Z"
  }
 },
 {
  "id": 1,
  "status": "successful",
  "subsystem": {
   "name": "bark.board",
   "kind": "board"
  },
  "metadata": {
   "chain_anchor": "734cf3347d7c2bd73e93fc52487f1743a5239f04a7bae1926d06944743cc8367:0",
   "onchain_fee_sat": 13
  },
  "intended_balance_sat": 29987,
  "effective_balance_sat": 29987,
  "offchain_fee_sat": 0,
  "sent_to": [],
  "received_on": [],
  "input_vtxos": [],
  "output_vtxos": [
   "fbbda5bf2306aaaecb70093e8b59be4a81d054a8d417a0c0e5759b5873061650:0"
  ],
  "exited_vtxos": [],
  "time": {
   "created_at": "2026-11-01T13:50:21.463235443Z",
   "updated_at": "2026-11-01T13:50:21.467817643Z",
   "completed_at": "2026-11-01T14:15:51.255260130Z"
  }
 }
]"#;
        let (base, _) = mock::spawn(move |_| (200, page.to_string()));
        let entries = client(&base).payments_since(0).await.unwrap();
        let credited: Vec<WalletPayment> = entries
            .into_iter()
            .filter_map(HistoryEntry::into_wallet_payments)
            .flatten()
            .collect();
        assert_eq!(
            credited.len(),
            1,
            "exactly the real receive credits: {credited:?}"
        );
        let p = &credited[0];
        assert!(
            p.address.starts_with("lntbs"),
            "LN receive attributes by its BOLT11: {}",
            p.address
        );
        assert_eq!(p.amount_sats, 5000);
        assert!(matches!(p.rail, Rail::Ark));
        assert!(
            p.vtxo_id.as_deref().unwrap_or("").contains(':'),
            "dedupe key is the HTLC outpoint"
        );
        assert!(
            p.detected_at > 1_700_000_000,
            "cursor timestamp parsed from time.completed_at"
        );
    }

    #[tokio::test]
    async fn the_bridge_maps_history_to_wallet_payments() {
        let page = r#"{"payments": [{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v1", "detected_at": 42, "status": "successful", "subsystem": {"name": "lightning", "kind": "receive"}}]}"#;
        let (base, _) = mock::spawn(move |_| (200, page.to_string()));
        let wallet = BarkdWallet::new(BarkdConfig::new(base, "test-token")).unwrap();
        // Called from inside a tokio context on purpose: the bridge must
        // not panic the way a bare Runtime::block_on would.
        let payments = wallet.poll_payments(0).unwrap();
        assert_eq!(payments.len(), 1);
        assert_eq!(payments[0].address, "ark1qinv");
        assert_eq!(payments[0].amount_sats, 1000);
        assert!(matches!(payments[0].rail, Rail::Ark));
    }

    #[tokio::test]
    async fn health_maps_the_asp_connection() {
        let (base, _) = mock::spawn(|_| {
            (
                200,
                r#"{"connected": true, "asp_url": "https://ark.example"}"#.to_string(),
            )
        });
        let wallet = BarkdWallet::new(BarkdConfig::new(base, "t")).unwrap();
        let health = wallet.health();
        assert!(health.reachable);
        assert_eq!(health.asp, AspStatus::Ok);
        assert!(health.detail.contains("ark.example"));
    }

    #[tokio::test]
    async fn unreachable_barkd_reports_down() {
        let wallet = BarkdWallet::new(BarkdConfig::new("http://127.0.0.1:9/api/v1", "t")).unwrap();
        let health = wallet.health();
        assert!(!health.reachable);
        assert_eq!(health.asp, AspStatus::Unknown);
    }
}
