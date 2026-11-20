//! Easy Bitcoin Payments — Phase 1 code-complete, EXPERIMENTAL.
//!
//! A free, open-source, self-hosted Bitcoin payments server for online stores.
//! The payment loop is implemented against a barkd 0.9.0 sidecar: invoice API,
//! hosted checkout, polling watcher with a persisted cursor, VTXO expiry queue,
//! settlement engine, setup wizard, and signed webhooks. This build is
//! EXPERIMENTAL. Signet (play money) is the default; mainnet is opt-in with
//! `EBP_MODE=live` and is for very small amounts only.

pub mod admin_auth;
pub mod aead;
pub mod alerts;
pub mod audit;
pub mod auth;
pub mod backup;
pub mod config;
pub mod db;
pub mod emergency_exit;
pub mod error;
pub mod health_checks;
pub mod invoice;
pub mod managed_barkd;
pub mod models;
pub mod passkey;
pub mod payout;
pub mod ratelimit;
pub mod rates;
pub mod reconcile;
pub mod retention;
pub mod routes;
pub mod secrets;
pub mod settings;
pub mod shopkey;
pub mod store;
pub mod totp;
pub mod updates;
pub mod util;
pub mod wallet;
pub mod watcher;
pub mod weak_password;
pub mod webhook;
pub mod webhook_outbox;

/// Crate version, surfaced by `GET /api/v1/health`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
