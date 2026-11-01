#![allow(dead_code)] // create_wallet / refresh_all / client() are consumed by
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
//! [`BarkdConfig::base_url`]). Response *shapes* are not pinned: the schema
//! dumps for every endpoint we use are empty, so each parsed response carries
//! a `VERIFY: field shape` marker and parses defensively — a missing
//! safety-critical field fails loudly instead of silently reading zero.
//! Close the markers when `docs/barkd-openapi.json` is filled in and the
//! drift CI (plan task 2) runs.
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
//! logged. Never call GET /wallet/mnemonic: --expose-mnemonic stays off.

use std::future::Future;
use std::fmt;
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
// are NOT pinned: the schema dumps for these endpoints are empty, so every
// response parser below carries a `VERIFY: field shape` marker.
// ---------------------------------------------------------------------------
/// Create (or restore) the wallet. POST.
const PATH_CREATE_WALLET: &str = "/wallet/create";
/// Fresh Ark receive address. POST.
const PATH_ADDRESS_NEXT: &str = "/wallet/addresses/next";
/// Fresh on-chain receive address. POST.
const PATH_ONCHAIN_ADDRESS_NEXT: &str = "/onchain/addresses/next";
/// On-chain and off-chain balances. GET.
const PATH_BALANCE: &str = "/wallet/balance";
/// Confirmed on-chain balance — the emergency-exit reserve. GET.
const PATH_ONCHAIN_BALANCE: &str = "/onchain/balance";
/// VTXO list with per-VTXO expiry. GET.
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
const PATH_CONNECTED: &str = "/wallet/connected";

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
fn is_loopback_url(url: &str) -> bool {
    url.contains("://127.0.0.1") || url.contains("://localhost") || url.contains("://[::1]")
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
        let mut value = HeaderValue::from_str(&format!("Bearer {}", config.token())).map_err(
            |_| {
                WalletError::Rejected(
                    "barkd token contains characters that are invalid in an HTTP header"
                        .to_string(),
                )
            },
        )?;
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
        flex::str_field(&v, &["address", "onchain_address", "bitcoin_address"]).ok_or_else(
            || {
                WalletError::Rejected(
                    "barkd address response had no address field (VERIFY: field shape)"
                        .to_string(),
                )
            },
        )
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
        let spendable = flex::i64_field(&v, &["spendable_sat", "spendable"])
            .ok_or_else(|| {
                WalletError::Rejected(
                    "barkd balance had no spendable_sat field (VERIFIED shape)".to_string(),
                )
            })?;
        let claimable =
            flex::i64_field(&v, &["claimable_lightning_receive_sat"]).unwrap_or(0);
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
    // VERIFY: field shape — the /wallet/vtxos dump is empty in the pinned
    // extract. Primary model: {"spendable": [...], "pending": [...]}; a flat
    // array is accepted defensively (all VTXOs spendable).
    pub async fn vtxos(&self) -> Result<VtxoSet, WalletError> {
        let v: Json = self.get(PATH_VTXOS).await?;
        let spendable = flex::array_field(&v, &["spendable", "spendable_vtxos", "vtxos"])
            .ok_or_else(|| {
                WalletError::Rejected(
                    "barkd vtxo list had no array (VERIFY: field shape)".to_string(),
                )
            })?;
        let pending = v
            .get("pending")
            .and_then(Json::as_array)
            .or_else(|| v.get("pending_vtxos").and_then(Json::as_array))
            .cloned()
            .unwrap_or_default();
        let parse = |items: Vec<Json>| -> Result<Vec<Vtxo>, WalletError> {
            items
                .into_iter()
                .map(|item| {
                    serde_json::from_value(item).map_err(|e| {
                        WalletError::Rejected(format!(
                            "unparseable vtxo entry: {e} (VERIFY: field shape)"
                        ))
                    })
                })
                .collect()
        };
        Ok(VtxoSet {
            spendable: parse(spendable)?,
            pending: parse(pending)?,
        })
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
            let height = v.expiry_height;
            let expires_at = v.expires_at.unwrap_or_else(|| match (&tip, height) {
                (Some(tip), Some(height)) if height as i64 > tip.height => {
                    tip.timestamp + (height as i64 - tip.height) * 600
                }
                _ => 0, // already past the tip, or unknown: treat as urgent
            });
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
        let height = flex::i64_field(&v, &["height", "block_height", "tip"]).ok_or_else(|| {
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
        tracing::warn!(
            pages = MAX_HISTORY_PAGES,
            "barkd history exceeded the page cap; returning partial history"
        );
        Ok(out)
    }

    // -- Settlement ---------------------------------------------------------

    /// Ark send to a same-ASP destination (settlement sweep to the
    /// merchant's Ark wallet). Returns barkd's raw receipt: its shape is
    /// not pinned (VERIFY: field shape), and never parsing it means a
    /// successful send cannot be mistaken for a failed one.
    pub async fn ark_send(&self, destination: &str, amount_sats: i64) -> Result<Json, WalletError> {
        // VERIFY: field shape — the /wallet/send dump is empty in the pinned
        // extract; destination/amount_sats is our best-effort model.
        let body =
            serde_json::json!({ "destination": destination, "amount_sats": amount_sats });
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
        let body = serde_json::json!({ "vtxo_ids": vtxo_ids });
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
        let body = serde_json::json!({ "amount_sats": amount_sats, "description": description });
        let v: Json = self.post(PATH_LN_INVOICE, &body).await?;
        let bolt11 = flex::str_field(&v, &["invoice", "payment_request", "bolt11", "ln_invoice"])
            .ok_or_else(|| {
                WalletError::Rejected(
                    "barkd lightning invoice had no bolt11 field (VERIFY: field shape)"
                        .to_string(),
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
    #[serde(default, alias = "outpoint", alias = "vtxo_id")]
    pub id: String,
    #[serde(default, alias = "amount", alias = "sats")]
    pub amount_sats: i64,
    #[serde(default, alias = "expiry", alias = "expires_at_height")]
    pub expiry_height: Option<u64>,
    #[serde(default, alias = "expiry_timestamp")]
    pub expires_at: Option<i64>,
}

/// One entry of `GET /wallet/history`.
// VERIFY: field shape — the dump is empty in the pinned extract; the fields
// below are the minimum the watcher needs (attribution address, amount, a
// dedupe key, a timestamp). /wallet/movements is the documented fallback if
// this shape does not fit.
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
    #[serde(default, alias = "created_at", alias = "timestamp", alias = "time")]
    pub detected_at: Option<i64>,
    #[serde(default)]
    pub confirmed_at: Option<i64>,
    /// Movement direction/kind, when barkd reports one.
    #[serde(default, alias = "kind", alias = "type")]
    pub direction: Option<String>,
}

impl HistoryEntry {
    /// Map to the watcher's payment shape. `None` when the entry cannot be
    /// attributed (no address), cannot be deduped (no VTXO id and no txid),
    /// is an outbound movement, or is not a positive receive — never credit
    /// an invoice from those.
    pub fn into_wallet_payment(self) -> Option<WalletPayment> {
        let direction = self
            .direction
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        if ["send", "sent", "offboard", "exit"]
            .iter()
            .any(|d| direction.contains(d))
        {
            return None;
        }
        if self.amount_sats <= 0 {
            return None;
        }
        let address = self.address?;
        if self.vtxo_id.is_none() && self.txid.is_none() {
            return None;
        }
        Some(WalletPayment {
            address,
            // LN-received VTXOs also carry a vtxo_id and settle under the
            // same confirmation rule; distinguishing them needs a
            // discriminator in barkd's history payload (VERIFY: field shape).
            rail: if self.vtxo_id.is_some() {
                Rail::Ark
            } else {
                Rail::Onchain
            },
            amount_sats: self.amount_sats,
            vtxo_id: self.vtxo_id,
            txid: self.txid,
            detected_at: self.detected_at.unwrap_or_else(crate::util::now_unix),
            confirmed_at: self.confirmed_at,
        })
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
        &["trusted_spendable_sat", "confirmed_sat", "total_sat", "spendable"],
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
            WalletError::Rejected(
                "barkd history had no list (VERIFY: field shape)".to_string(),
            )
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
                WalletError::Unavailable(format!(
                    "could not spawn the barkd bridge thread: {err}"
                ))
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

    fn block_on<T: Send + 'static, F: Future<Output = T> + Send + 'static>(
        &self,
        fut: F,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |rt| {
            let _ = tx.send(rt.block_on(fut));
        });
        let sender = self
            .tx
            .lock()
            .ok()
            .unwrap_or_else(|| panic!("barkd bridge poisoned"));
        sender
            .send(job)
            .unwrap_or_else(|_| panic!("the barkd bridge thread has exited"));
        rx.recv()
            .unwrap_or_else(|_| panic!("the barkd bridge dropped a job without answering"))
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
        let entries = self
            .block_on(async move { client.payments_since(since).await })?;
        Ok(entries
            .into_iter()
            .filter_map(HistoryEntry::into_wallet_payment)
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

    fn read_request(
        stream: &mut TcpStream,
    ) -> Option<(String, String, Option<String>, String)> {
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
        let (base, recorded) = mock::spawn(|_| {
            (
                200,
                r#"{"spendable_sat": 2}"#.to_string(),
            )
        });
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
        let (base, _) = mock::spawn(|_| {
            (
                401,
                r#"{"error":"bad token test-token"}"#.to_string(),
            )
        });
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
        let (base, _) = mock::spawn(|_| {
            (
                400,
                "invalid request for test-token".to_string(),
            )
        });
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
                r#"{"total_sat": 30500, "trusted_spendable_sat": 30000, "confirmed_sat": 30000}"#.to_string(),
            )
        });
        let b = client(&base).onchain_balance().await.unwrap();
        assert_eq!(b.confirmed_sats, 30_000);
        assert_eq!(b.unconfirmed_sats, 500);
    }

    #[tokio::test]
    async fn parses_vtxos_in_both_shapes() {
        let split = r#"{"spendable": [{"id": "a", "amount_sats": 100, "expires_at": 1750000000}], "pending": [{"id": "b", "amount_sats": 50, "expiry_height": 900}]}"#;
        let (base, _) = mock::spawn(move |_| (200, split.to_string()));
        let set = client(&base).vtxos().await.unwrap();
        assert_eq!(set.spendable.len(), 1);
        assert_eq!(set.pending.len(), 1);
        assert_eq!(set.spendable[0].id, "a");
        assert_eq!(set.pending[0].expiry_height, Some(900));

        let flat = r#"[{"id": "c", "amount_sats": 7}]"#;
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
            "/api/v1/bitcoin/tip" => (
                200,
                r#"{"height": 800, "timestamp": 1000000}"#.to_string(),
            ),
            _ => (404, "{}".to_string()),
        });
        let queue = client(&base).vtxo_queue().await.unwrap();
        assert_eq!(queue.len(), 1);
        // 100 blocks * 600s after the tip timestamp.
        assert_eq!(queue[0].expires_at, 1_000_000 + 100 * 600);
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
        assert!(all[0].path.contains("since=99"), "first page: {}", all[0].path);
        assert!(
            all[1].path.contains("cursor=cursor-2"),
            "second page: {}",
            all[1].path
        );
    }

    #[test]
    fn history_entries_map_to_wallet_payments() {
        let entry = |json: &str| serde_json::from_str::<HistoryEntry>(json).unwrap();
        let ark = entry(
            r#"{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v1", "detected_at": 10}"#,
        )
        .into_wallet_payment()
        .unwrap();
        assert_eq!(ark.address, "ark1qinv");
        assert_eq!(ark.amount_sats, 1000);
        assert!(matches!(ark.rail, Rail::Ark));

        let onchain = entry(
            r#"{"address": "bc1qinv", "amount_sats": 5, "txid": "t", "created_at": 11}"#,
        )
        .into_wallet_payment()
        .unwrap();
        assert!(matches!(onchain.rail, Rail::Onchain));
        assert_eq!(onchain.detected_at, 11);

        // Outbound movements, unattributable entries, entries without a
        // dedupe key, and non-positive amounts are never credited.
        assert!(entry(
            r#"{"address": "a", "amount_sats": 1, "vtxo_id": "v", "detected_at": 1, "kind": "send"}"#
        )
        .into_wallet_payment()
        .is_none());
        assert!(entry(r#"{"amount_sats": 1, "vtxo_id": "v", "detected_at": 1}"#)
            .into_wallet_payment()
            .is_none());
        assert!(entry(r#"{"address": "a", "amount_sats": 1, "detected_at": 1}"#)
            .into_wallet_payment()
            .is_none());
        assert!(entry(
            r#"{"address": "a", "amount_sats": 0, "vtxo_id": "v", "detected_at": 1}"#
        )
        .into_wallet_payment()
        .is_none());
    }

    #[tokio::test]
    async fn ark_send_posts_destination_and_amount() {
        let (base, recorded) = mock::spawn(|_| (200, r#"{"sent": true}"#.to_string()));
        client(&base).ark_send("ark1merchant", 50_000).await.unwrap();
        let all = recorded.lock().unwrap();
        assert_eq!(all[0].method, "POST");
        assert_eq!(all[0].path, "/api/v1/wallet/send");
        let body: serde_json::Value = serde_json::from_str(&all[0].body).unwrap();
        assert_eq!(body["destination"], "ark1merchant");
        assert_eq!(body["amount_sats"], 50_000);
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
        assert_eq!(body["amount_sats"], 21_000);
    }

    #[tokio::test]
    async fn the_bridge_maps_history_to_wallet_payments() {
        let page = r#"{"payments": [{"address": "ark1qinv", "amount_sats": 1000, "vtxo_id": "v1", "detected_at": 42}]}"#;
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