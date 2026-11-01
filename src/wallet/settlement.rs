//! Settlement engine (Phase 1, Custody Level 0).
//!
//! Three jobs, in priority order:
//!
//! 1. **Renewal** — refresh spendable VTXOs inside the renewal window,
//!    before their per-VTXO expiry (learn_lifetime.md). Renewal never moves
//!    funds.
//! 2. **Sweep** — move the excess above `hot_wallet_cap_sats` to the
//!    merchant's own wallet: an Ark send when the destination is on the
//!    same ASP (e.g. their Arké wallet), or a cooperative offboard of the
//!    whole till when it is an on-chain address (learn_offboard.md).
//!    Emergency exits are never automated — they are a documented runbook.
//! 3. **Exit reserve** — keep at least [`ONCHAIN_RESERVE_PER_VTXO_SATS`]
//!    confirmed on-chain sats per spendable VTXO, checked against
//!    `GET /onchain/balance`, so an emergency exit is always fundable. A
//!    sweep that cannot keep this reserve is refused and logged: the VTXOs
//!    being swept are exactly the VTXOs that would need an emergency exit
//!    if the ASP stalls instead of cooperating.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::config::Config;
use crate::db::Db;
use crate::error::AppError;
use crate::util::{new_id, now_unix};
use crate::wallet::barkd::{BarkdClient, ONCHAIN_RESERVE_PER_VTXO_SATS};

/// The on-chain emergency-exit reserve for the VTXOs the till holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitReserve {
    pub onchain_sats: i64,
    pub vtxo_count: usize,
    pub required_sats: i64,
    pub ok: bool,
    pub shortfall_sats: i64,
}

/// Compute the exit reserve for `onchain_sats` of confirmed on-chain coins
/// and `vtxo_count` spendable VTXOs. Emergency exits need on-chain funds to
/// CPFP their way out (learn_exit.md: >= 15,000 sats per VTXO, 1-2 hours),
/// so the till must always hold this much on-chain.
pub fn exit_reserve(onchain_sats: i64, vtxo_count: usize) -> ExitReserve {
    let required_sats = ONCHAIN_RESERVE_PER_VTXO_SATS * vtxo_count as i64;
    ExitReserve {
        onchain_sats,
        vtxo_count,
        required_sats,
        ok: onchain_sats >= required_sats,
        shortfall_sats: (required_sats - onchain_sats).max(0),
    }
}

/// What one settlement tick did, for logs and tests.
#[derive(Debug, Default)]
pub struct TickReport {
    pub refreshed_vtxos: usize,
    pub swept_sats: Option<i64>,
    pub sweep_refused: Option<String>,
}

pub struct SettlementEngine {
    client: BarkdClient,
    db: Db,
    config: Arc<Config>,
}

impl SettlementEngine {
    pub fn new(client: BarkdClient, db: Db, config: Arc<Config>) -> Self {
        Self { client, db, config }
    }

    /// Run forever on the configured interval.
    pub async fn run(self) {
        let mut ticker = tokio::time::interval(Duration::from_secs(
            self.config.settlement_interval_secs.max(1),
        ));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            match self.tick().await {
                Ok(report)
                    if report.refreshed_vtxos > 0
                        || report.swept_sats.is_some()
                        || report.sweep_refused.is_some() =>
                {
                    tracing::info!(
                        refreshed = report.refreshed_vtxos,
                        swept_sats = ?report.swept_sats,
                        "settlement tick acted"
                    );
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "settlement tick failed"),
            }
        }
    }

    /// One settlement cycle.
    pub async fn tick(&self) -> Result<TickReport, AppError> {
        let mut report = TickReport::default();

        let Some(destination) = self
            .config
            .settlement_destination
            .clone()
            .filter(|d| !d.trim().is_empty())
        else {
            return Ok(report);
        };

        let vtxos = self.client.vtxos().await.map_err(AppError::from)?;
        let onchain = self.client.onchain_balance().await.map_err(AppError::from)?;
        let balances = self.client.balances().await.map_err(AppError::from)?;

        let now = now_unix();
        let lead = self.config.renewal_lead_time_secs as i64;

        // 1. Renewal: refresh spendable VTXOs inside the renewal window,
        //    before their per-VTXO expiry. A VTXO barkd already refreshed
        //    drops out of the window by itself, so this stays quiet when
        //    renewal is already handled.
        let due: Vec<_> = vtxos
            .spendable
            .iter()
            .filter(|v| v.expires_at.map(|t| t - now <= lead).unwrap_or(true))
            .collect();
        let due_ids: Vec<String> = due
            .iter()
            .map(|v| v.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        if !due_ids.is_empty() {
            match self.client.refresh_vtxos(&due_ids).await {
                Ok(_) => {
                    let amount: i64 = due.iter().map(|v| v.amount_sats).sum();
                    let count = due_ids.len();
                    let dest = destination.clone();
                    self.db
                        .call(move |conn| {
                            record(
                                conn,
                                "renew",
                                amount,
                                &dest,
                                "broadcast",
                                Some(&format!("refreshed {count} vtxos ahead of expiry")),
                            )
                        })
                        .await?;
                    report.refreshed_vtxos = count;
                }
                Err(err) => tracing::warn!(error = %err, "vtxo refresh failed"),
            }
        }

        // 2. Cap-driven sweep: move the excess above the hot cap to the
        //    merchant's own wallet.
        let offchain = balances.offchain_sats;
        let excess = offchain.saturating_sub(self.config.hot_wallet_cap_sats);
        if excess <= 0 {
            return Ok(report);
        }

        // Ark addresses are network-prefixed: `ark1…` (mainnet),
        // `tark1…` (signet). Route either to the Ark-send path; anything
        // else is treated as an on-chain destination.
        let ark = destination.starts_with("ark1") || destination.starts_with("tark1");
        let kind = if ark { "sweep_ark" } else { "sweep_onchain" };

        // 3. Exit-reserve gate, checked against /onchain/balance.
        let reserve = exit_reserve(onchain.confirmed_sats, vtxos.spendable.len());
        if !reserve.ok {
            let reason = format!(
                "exit reserve short: {}/{} sats for {} spendable vtxos — fund the till's on-chain wallet",
                reserve.onchain_sats, reserve.required_sats, reserve.vtxo_count
            );
            tracing::warn!(%reason, "sweep refused");
            report.sweep_refused = Some(reason.clone());
            let dest = destination.clone();
            self.db
                .call(move |conn| record(conn, kind, excess, &dest, "failed", Some(&reason)))
                .await?;
            return Ok(report);
        }

        let (amount, result) = if ark {
            (excess, self.client.ark_send(&destination, excess).await)
        } else {
            // On-chain destinations offboard the whole till; partial
            // on-chain sweeps (/wallet/offboard/vtxos) are a Phase 1 leftover.
            (offchain, self.client.offboard_all(&destination).await)
        };

        match result {
            Ok(receipt) => {
                let detail = serde_json::to_string(&receipt).unwrap_or_else(|_| "{}".to_string());
                let dest = destination.clone();
                self.db
                    .call(move |conn| record(conn, kind, amount, &dest, "broadcast", Some(&detail)))
                    .await?;
                report.swept_sats = Some(amount);
                tracing::info!(kind, amount_sats = amount, "settlement sweep sent");
            }
            Err(err) => {
                let reason = err.to_string();
                tracing::warn!(error = %reason, "settlement sweep failed");
                let dest = destination.clone();
                self.db
                    .call(move |conn| record(conn, kind, amount, &dest, "failed", Some(&reason)))
                    .await?;
            }
        }

        Ok(report)
    }
}

fn record(
    conn: &Connection,
    kind: &str,
    amount_sats: i64,
    destination: &str,
    status: &str,
    detail: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_id("stl"),
            kind,
            amount_sats,
            destination,
            status,
            detail,
            now_unix()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::barkd::mock;

    fn test_config(destination: Option<&str>, cap: i64) -> Arc<Config> {
        let mut c = Config::default();
        c.settlement_destination = destination.map(str::to_string);
        c.hot_wallet_cap_sats = cap;
        c.renewal_lead_time_secs = 604_800;
        Arc::new(c)
    }

    #[test]
    fn exit_reserve_math() {
        let exact = exit_reserve(15_000, 1);
        assert!(exact.ok);
        assert_eq!(exact.shortfall_sats, 0);

        let short = exit_reserve(14_999, 1);
        assert!(!short.ok);
        assert_eq!(short.shortfall_sats, 1);

        let none = exit_reserve(0, 0);
        assert!(none.ok, "no vtxos, no reserve needed");

        let two = exit_reserve(29_999, 2);
        assert!(!two.ok);
        assert_eq!(two.shortfall_sats, 1);
    }

    #[test]
    fn sweepable_onchain_sats_keeps_the_reserve() {
        use crate::wallet::barkd::sweepable_onchain_sats;
        assert_eq!(sweepable_onchain_sats(50_000, 2), 20_000);
        assert_eq!(sweepable_onchain_sats(10_000, 2), 0);
        assert_eq!(sweepable_onchain_sats(30_000, 2), 0);
    }

    #[tokio::test]
    async fn sweeps_excess_over_the_cap_via_ark_send() {
        let (base, recorded) = mock::spawn(|req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (
                200,
                r#"{"spendable_sat": 150000}"#.to_string(),
            ),
            "/api/v1/wallet/send" => (200, r#"{"sent": true}"#.to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(crate::wallet::barkd::BarkdConfig::new(&base, "t")).unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("ark1merchant"), 100_000),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(50_000));

        let requests = recorded.lock().unwrap();
        let send = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/send")
            .expect("send recorded");
        assert_eq!(send.method, "POST");
        let body: serde_json::Value = serde_json::from_str(&send.body).unwrap();
        assert_eq!(body["destination"], "ark1merchant");
        assert_eq!(body["amount_sats"], 50_000);

        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_ark' AND status = 'broadcast' AND amount_sats = 50000",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn onchain_destinations_offboard_the_whole_till() {
        let (base, recorded) = mock::spawn(|req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (
                200,
                r#"{"spendable_sat": 150000}"#.to_string(),
            ),
            "/api/v1/wallet/offboard/all" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(crate::wallet::barkd::BarkdConfig::new(&base, "t")).unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("bc1qmerchant"), 100_000),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));

        let requests = recorded.lock().unwrap();
        let offboard = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/offboard/all")
            .expect("offboard recorded");
        assert_eq!(offboard.method, "POST");
        let body: serde_json::Value = serde_json::from_str(&offboard.body).unwrap();
        assert_eq!(body["address"], "bc1qmerchant");
    }

    #[tokio::test]
    async fn refuses_a_sweep_that_would_breach_the_exit_reserve() {
        // Two spendable VTXOs need 30,000 sats of reserve; the till holds
        // 10,000 confirmed on-chain. The sweep must be refused and logged.
        let (base, recorded) = mock::spawn(|req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 200000, "expires_at": 9999999999, "state": {"type": "spendable"}}, {"id": "v2", "amount_sat": 1, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 10000, "trusted_spendable_sat": 10000}"#.to_string()),
            "/api/v1/wallet/balance" => (
                200,
                r#"{"spendable_sat": 200001}"#.to_string(),
            ),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(crate::wallet::barkd::BarkdConfig::new(&base, "t")).unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("bc1qmerchant"), 100_000),
        );
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        let reason = report.sweep_refused.expect("refusal reason");
        assert!(reason.contains("reserve"), "{reason}");

        let requests = recorded.lock().unwrap();
        assert!(
            requests.iter().all(|r| r.method == "GET"),
            "no sweep may be sent while the reserve is short"
        );

        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE status = 'failed'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn refreshes_vtxos_inside_the_renewal_window() {
        let soon = crate::util::now_unix() + 86_400; // expires tomorrow
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 1000, "expires_at": {soon}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 15000, "trusted_spendable_sat": 15000}"#.to_string()),
            "/api/v1/wallet/balance" => (
                200,
                r#"{"spendable_sat": 1000}"#.to_string(),
            ),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(crate::wallet::barkd::BarkdConfig::new(&base, "t")).unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("ark1merchant"), 100_000),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 1);
        assert!(report.swept_sats.is_none(), "below the cap: no sweep");

        let requests = recorded.lock().unwrap();
        let refresh = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/refresh/vtxos")
            .expect("refresh recorded");
        assert_eq!(refresh.method, "POST");
        let body: serde_json::Value = serde_json::from_str(&refresh.body).unwrap();
        assert_eq!(body["vtxo_ids"][0], "v1");
    }

    #[tokio::test]
    async fn without_a_destination_the_engine_stays_idle() {
        let (base, recorded) = mock::spawn(|_| (200, "{}".to_string()));
        let client = crate::wallet::barkd::BarkdClient::new(crate::wallet::barkd::BarkdConfig::new(&base, "t")).unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = SettlementEngine::new(client, db, test_config(None, 100_000));
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report.sweep_refused.is_none());
        assert!(recorded.lock().unwrap().is_empty());
    }
}