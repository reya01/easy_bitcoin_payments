#![allow(dead_code)] // Phase 1 skeleton; wired into WalletAdapter when it goes async.

//! Phase 1: `WalletAdapter` over the barkd sidecar's REST API.
//!
//! Docs: barkd_index.md (why barkd), barkd_start.md (base URL + OpenAPI),
//! barkd_authenticate.md (bearer token), connection-details.md (ASP/Esplora).
//! Paths are validated against the pinned spec `docs/barkd-openapi.json`
//! (from GET /api-docs/openapi.json); CI fails on drift with the pinned barkd.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::de::DeserializeOwned;

use super::WalletError;

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

    // Watcher surface: per-address receive + cursored payment history.
    // Settlement surface: balances, VTXO list, ark send / offboard.
    // Never call GET /wallet/mnemonic: --expose-mnemonic stays off.

    /// Payments received at one invoice address.
    pub async fn received_on(&self, address: &str) -> Result<Vec<PaymentEntry>, WalletError> {
        self.get(&format!("/payments?address={address}")).await
    }

    /// Payment history at or after `cursor` (UNIX seconds), oldest first.
    pub async fn payments_since(&self, cursor: i64) -> Result<Vec<PaymentEntry>, WalletError> {
        self.get(&format!("/payments?since={cursor}")).await
    }

    /// On-chain and off-chain balances, tracked separately: the on-chain
    /// part is the emergency-exit CPFP reserve.
    pub async fn balances(&self) -> Result<Balances, WalletError> {
        self.get("/wallet/balance").await
    }

    /// VTXO list with per-VTXO expiry (drives refresh scheduling).
    pub async fn vtxos(&self) -> Result<Vec<Vtxo>, WalletError> {
        self.get("/wallet/vtxos").await
    }

    /// Ark send to an Ark address (settlement sweep).
    pub async fn ark_send(&self, dest: &str, sats: i64) -> Result<Json, WalletError> {
        let body = serde_json::json!({"destination": dest, "amount_sats": sats});
        self.post("/wallet/send", &body).await
    }

    /// Offboard on-chain. Callers keep >= [`ONCHAIN_RESERVE_PER_VTXO_SATS`]
    /// per remaining VTXO as the emergency-exit CPFP reserve.
    pub async fn offboard(&self, dest: &str, sats: i64) -> Result<Json, WalletError> {
        let body = serde_json::json!({"destination": dest, "amount_sats": sats});
        self.post("/wallet/offboard", &body).await
    }
}

/// JSON receipt from barkd send/offboard (txid, status, ...).
pub type Json = serde_json::Value;

/// On-chain sats kept per VTXO so an emergency exit can always be CPFP'd.
pub const ONCHAIN_RESERVE_PER_VTXO_SATS: i64 = 15_000;

/// On-chain sats a settlement sweep may move while preserving the reserve.
pub fn sweepable_onchain_sats(onchain_sats: i64, vtxo_count: usize) -> i64 {
    (onchain_sats - ONCHAIN_RESERVE_PER_VTXO_SATS * vtxo_count as i64).max(0)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PaymentEntry {
    pub address: String,
    pub amount_sats: i64,
    #[serde(default)]
    pub vtxo_id: Option<String>,
    #[serde(default)]
    pub txid: Option<String>,
    pub detected_at: i64,
    #[serde(default)]
    pub confirmed_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Balances {
    pub onchain_sats: i64,
    pub offchain_sats: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Vtxo {
    pub id: String,
    pub amount_sats: i64,
    pub expires_at: i64, // UNIX seconds; refresh before this.
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn mock_barkd(body: &'static str) -> (String, tokio::sync::mpsc::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).await.unwrap();
            let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
            sock.write_all(format!("{head}{body}").as_bytes()).await.unwrap();
        });
        (format!("http://{addr}"), rx)
    }

    #[tokio::test]
    async fn client_parses_shapes_and_sends_bearer_token() {
        let (base, mut rx) = mock_barkd(r#"{"onchain_sats":45000,"offchain_sats":120000}"#).await;
        let w = BarkdWallet::new(BarkdConfig { base_url: base, token: "s3cret".into() }).unwrap();
        let b = w.balances().await.unwrap();
        assert_eq!((b.onchain_sats, b.offchain_sats), (45_000, 120_000));
        let req = rx.recv().await.unwrap();
        assert!(req.contains("GET /wallet/balance"));
        assert!(req.contains("authorization: Bearer s3cret"));
    }

    #[test]
    fn settlement_keeps_per_vtxo_onchain_reserve() {
        assert_eq!(sweepable_onchain_sats(100_000, 2), 70_000);
        assert_eq!(sweepable_onchain_sats(30_000, 2), 0);
    }
}