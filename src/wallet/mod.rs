//! Wallet adapter boundary.
//!
//! Everything the payments server needs from a Bitcoin wallet lives behind
//! [`WalletAdapter`]. Phase 0 ships [`StubWallet`], which derives deterministic
//! placeholder addresses and observes no payments. Phase 1 replaces it with
//! [`barkd::BarkdWallet`], a reqwest client of the barkd sidecar's REST API
//! (docs: barkd_index.md) — no other module changes. bark-wallet is NOT
//! embedded: barkd owns the wallet, exits, and backups; EBP schedules
//! refreshes via its API.

use crate::models::Rail;

pub mod barkd;
pub mod settlement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveAddresses {
    pub ark_address: String,
    pub onchain_address: String,
}

#[derive(Debug, Clone)]
pub struct WalletPayment {
    /// The address the funds arrived at. Used to match the invoice.
    pub address: String,
    pub rail: Rail,
    pub amount_sats: i64,
    pub vtxo_id: Option<String>,
    pub txid: Option<String>,
    pub detected_at: i64,
    pub confirmed_at: Option<i64>,
}

/// An output paying one of the watched on-chain addresses, with its depth.
/// `confirmations` is 0 while the transaction is only in the mempool.
#[derive(Debug, Clone)]
pub struct OnchainObservation {
    pub address: String,
    pub txid: String,
    pub vout: u32,
    pub amount_sats: i64,
    pub confirmations: i64,
}

/// A VTXO currently held by the wallet, with its own expiry — the source of
/// the dashboard's expiry queue. Per-VTXO expiries are real, not estimated:
/// board/refresh VTXOs live ~28 days, Lightning-received VTXOs 1-3 days, and
/// spend VTXOs inherit their parent's expiry (learn_lifetime.md).
#[derive(Debug, Clone)]
pub struct WalletVtxo {
    /// barkd's VTXO id/outpoint.
    pub id: String,
    pub amount_sats: i64,
    /// UNIX seconds; 0 when unknown (the queue treats unknown as urgent).
    pub expires_at: i64,
    /// Heuristic flag: a lifetime of 1-3 days at first sight means the VTXO
    /// probably arrived via Lightning and belongs in the urgent queue.
    pub likely_lightning: bool,
}

/// A Lightning invoice created through the ASP's gateway.
#[derive(Debug, Clone)]
pub struct LightningInvoice {
    /// The BOLT11 payment request.
    pub bolt11: String,
    /// barkd's identifier for polling `GET /lightning/receives/{identifier}`.
    pub identifier: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspStatus {
    Ok,
    Degraded,
    Down,
    Unknown,
}

impl AspStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AspStatus::Ok => "ok",
            AspStatus::Degraded => "degraded",
            AspStatus::Down => "down",
            AspStatus::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WalletHealth {
    pub reachable: bool,
    pub balance_sats: Option<i64>,
    pub asp: AspStatus,
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("wallet unavailable: {0}")]
    Unavailable(String),
    #[error("wallet rejected the request: {0}")]
    Rejected(String),
}

/// The single integration surface for Bark.
///
/// // BARK INTEGRATION: decided — Phase 1 implements this trait with
/// // `barkd::BarkdWallet` over barkd's local REST API (bearer token, OpenAPI
/// // spec pinned at docs/barkd-openapi.json). barkd is Second's reference
/// // daemon for webshops/payment processors; the SDK is for on-device keys.
/// // The trait goes async in Phase 1; method semantics stay as documented here.
pub trait WalletAdapter: Send + Sync {
    /// Derive a fresh receive address pair for an invoice.
    fn receive_addresses(&self, invoice_id: &str) -> Result<ReceiveAddresses, WalletError>;

    /// Current spendable balance of the server till, in sats.
    fn balance_sats(&self) -> Result<i64, WalletError>;

    /// Payments observed at or after `since` (UNIX seconds).
    fn poll_payments(&self, since: i64) -> Result<Vec<WalletPayment>, WalletError>;

    /// Outputs paying any of the `watch` on-chain addresses, mempool and
    /// mined, with their confirmation depth. Wallets without an on-chain
    /// view return nothing, which means on-chain payments are never
    /// credited (fail closed).
    fn poll_onchain(&self, watch: &[String]) -> Result<Vec<OnchainObservation>, WalletError> {
        let _ = watch;
        Ok(Vec::new())
    }

    /// Wallet and ASP reachability for `GET /api/v1/health`.
    fn health(&self) -> WalletHealth;

    /// VTXOs currently held, each with its own expiry — the source of the
    /// dashboard's expiry queue. Wallets that do not track VTXOs return an
    /// empty list.
    fn vtxos(&self) -> Result<Vec<WalletVtxo>, WalletError> {
        Ok(Vec::new())
    }

    /// Best-effort Lightning invoice via the ASP's gateway. Callers must
    /// degrade gracefully when this fails: checkout stays functional with
    /// Ark and on-chain alone.
    /// `expiry_secs`: how long the invoice may be paid for (the payment
    /// request's own lifetime, so nobody pays an expired order).
    fn lightning_invoice(
        &self,
        amount_sats: i64,
        description: &str,
        expiry_secs: i64,
    ) -> Result<LightningInvoice, WalletError> {
        let _ = (amount_sats, description, expiry_secs);
        Err(WalletError::Rejected(
            "this wallet cannot create lightning invoices".to_string(),
        ))
    }
}

/// Phase 0 placeholder wallet.
///
/// Addresses are deterministic functions of the invoice id so tests and the
/// checkout page have something stable to display. It never reports payments.
pub struct StubWallet;

impl StubWallet {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubWallet {
    fn default() -> Self {
        Self::new()
    }
}

fn derive(invoice_id: &str, prefix: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(invoice_id.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}{}", &digest[..40])
}

impl WalletAdapter for StubWallet {
    fn receive_addresses(&self, invoice_id: &str) -> Result<ReceiveAddresses, WalletError> {
        Ok(ReceiveAddresses {
            ark_address: derive(invoice_id, "ark1qstub"),
            onchain_address: derive(invoice_id, "bc1qstub"),
        })
    }

    fn balance_sats(&self) -> Result<i64, WalletError> {
        Ok(0)
    }

    fn poll_payments(&self, _since: i64) -> Result<Vec<WalletPayment>, WalletError> {
        Ok(Vec::new())
    }

    fn health(&self) -> WalletHealth {
        WalletHealth {
            reachable: true,
            balance_sats: Some(0),
            asp: AspStatus::Unknown,
            detail: "stub wallet (Phase 0) — no Bark connection yet".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_deterministic_and_distinct() {
        let wallet = StubWallet::new();
        let a = wallet.receive_addresses("inv_1").unwrap();
        let b = wallet.receive_addresses("inv_1").unwrap();
        let c = wallet.receive_addresses("inv_2").unwrap();

        assert_eq!(a, b);
        assert_ne!(a.ark_address, c.ark_address);
        assert_ne!(a.onchain_address, c.onchain_address);
        assert!(a.ark_address.starts_with("ark1qstub"));
        assert!(a.onchain_address.starts_with("bc1qstub"));
    }

    #[test]
    fn stub_observes_no_payments() {
        let wallet = StubWallet::new();
        assert!(wallet.poll_payments(0).unwrap().is_empty());
        assert_eq!(wallet.balance_sats().unwrap(), 0);
    }

    #[test]
    fn health_reports_unknown_asp() {
        let health = StubWallet::new().health();
        assert!(health.reachable);
        assert_eq!(health.asp, AspStatus::Unknown);
        assert_eq!(health.asp.as_str(), "unknown");
    }
}
