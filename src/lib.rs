//! Easy Bitcoin Payments — Phase 0 foundations.
//!
//! A free, open-source, self-hosted Bitcoin payments server for online stores.
//! Phase 0 ships the invoice API, a hosted checkout page, a polling payment
//! watcher skeleton, and a signed webhook emitter — all against a stub wallet
//! layer. Real Bark integration lands in Phase 1; every integration point is
//! marked with `// BARK INTEGRATION`.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod invoice;
pub mod models;
pub mod rates;
pub mod settings;
pub mod routes;
pub mod store;
pub mod util;
pub mod wallet;
pub mod watcher;
pub mod webhook;

/// Crate version, surfaced by `GET /api/v1/health`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");