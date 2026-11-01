#![allow(dead_code)] // Phase 1 skeleton; wired into WalletAdapter when it goes async.

//! Phase 1: `WalletAdapter` over the barkd sidecar's REST API.
//!
//! Docs: barkd_index.md (why barkd), barkd_start.md (base URL + OpenAPI),
//! barkd_authenticate.md (bearer token), connection-details.md (ASP/Esplora).
//! Paths are validated against the pinned spec `docs/barkd-openapi.json`
//! (from GET /api-docs/openapi.json); CI fails on drift with the pinned barkd.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::de::DeserializeOwned;

use super::{AspStatus, ReceiveAddresses, WalletError, WalletHealth, WalletPayment};

#[derive(Debug, Clone)]
pub struct BarkdConfig {
    /// Default http://127.0.0.1:3000/api/v1 — loopback / internal container
    /// network only; barkd serves plain HTTP (barkd_start.md).
    pub base_url: String,
    /// From `barkd --datadir <path> secret show`. Never logged.
    pub token: String,
}

impl BarkdConfig {
    pub fn from_env() -> Result<Self, WalletError> {
        let base_url = std::env::var("EBP_BARKD_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000/api/v1".into());
        let token = std::env::var("EBP_BARKD_TOKEN")
            .map_err(|_| WalletError::Unavailable("EBP_BARKD_TOKEN is not set".into()))?;
        Ok(Self { base_url, token })
    }
}

#[derive(Debug, Clone)]
pub struct BarkdWallet {
    http: reqwest::Client,
    base: String,
}

impl BarkdWallet {
    pub fn new(cfg: BarkdConfig) -> Result<Self, WalletError> {
        let mut headers = HeaderMap::new();
        let mut v = HeaderValue::from_str(&format!("Bearer {}", cfg.token))
            .map_err(|e| WalletError::Rejected(e.to_string()))?;
        v.set_sensitive(true);
        headers.insert(AUTHORIZATION, v);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| WalletError::Unavailable(e.to_string()))?;
        Ok(Self { http, base: cfg.base_url.trim_end_matches('/').into() })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, WalletError> {
        self.http.get(format!("{}{}", self.base, path)).send().await
            .map_err(|e| WalletError::Unavailable(e.to_string()))?
            .error_for_status().map_err(|e| WalletError::Rejected(e.to_string()))?
            .json().await.map_err(|e| WalletError::Rejected(e.to_string()))
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, body: &serde_json::Value) -> Result<T, WalletError> {
        self.http.post(format!("{}{}", self.base, path)).json(body).send().await
            .map_err(|e| WalletError::Unavailable(e.to_string()))?
            .error_for_status().map_err(|e| WalletError::Rejected(e.to_string()))?
            .json().await.map_err(|e| WalletError::Rejected(e.to_string()))
    }

    // Phase 1 wires `WalletAdapter` (made async) to these calls; exact paths
    // and response types come from the pinned OpenAPI spec:
    //   receive_addresses -> fresh Ark address + fresh on-chain address
    //   poll_payments     -> payment history; map to WalletPayment (vtxo_id/txid)
    //   balance_sats      -> off-chain balance; track on-chain separately as
    //                        the emergency-exit CPFP reserve (>=15k sats/VTXO)
    //   health            -> daemon status + ASP reachability -> AspStatus
    // Settlement engine (separate module): refresh (rounds), Ark send /
    // offboard (sweeps), exit start/progress/claim (emergency only).
    // Never call GET /api/v1/wallet/mnemonic: --expose-mnemonic stays off.
}