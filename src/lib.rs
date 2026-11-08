//! Easy Bitcoin Payments — Phase 1 code-complete, EXPERIMENTAL.
//!
//! A free, open-source, self-hosted Bitcoin payments server for online stores.
//! The payment loop is implemented against a barkd 0.8.0 sidecar: invoice API,
//! hosted checkout, polling watcher with a persisted cursor, VTXO expiry queue,
//! settlement engine, setup wizard, and signed webhooks. This build is
//! EXPERIMENTAL — use signet or regtest only, never mainnet or real funds.

pub mod admin_auth;
pub mod aead;
pub mod alerts;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod config;
pub mod db;
pub mod error;
pub mod health_checks;
pub mod invoice;
pub mod managed_barkd;
pub mod models;
pub mod onchain;
pub mod payout;
pub mod ratelimit;
pub mod rates;
pub mod reconcile;
pub mod retention;
pub mod routes;
pub mod secrets;
pub mod settings;
pub mod store;
pub mod totp;
pub mod updates;
pub mod uri;
pub mod util;
pub mod wallet;
pub mod watcher;
pub mod webhook;
pub mod webhook_outbox;

/// Crate version, surfaced by `GET /api/v1/health`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
