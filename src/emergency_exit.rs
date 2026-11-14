//! Emergency withdrawal without the Ark server (a unilateral exit).
//!
//! The owner always holds the keys to the server's Ark coins, so they can be
//! taken out on-chain even if the Ark server is gone or refuses to help. It
//! is slow (several on-chain transactions per coin, then a wait of at least
//! the exit delta, about a day on mainnet) and costly (each transaction pays
//! network fees, from the server's own CONFIRMED on-chain balance). So it is
//! a last resort: nothing here ever starts on its own, and the server refuses
//! an exit that is not worth it (owner rule, 12 November 2026: at least
//! 100,000 sats AND at least twice the projected fees).
//!
//! Pure rules live here; the barkd calls are in `wallet::barkd` and the HTTP
//! handlers in `routes::exit`.

use serde::Serialize;
use serde_json::Value as Json;

/// Off on real money until the owner's signet drill has passed (MAINNET_PLAN
/// PR G step 11). Flip to `true` in its own reviewed change after the drill.
pub const ENABLED_ON_MAINNET: bool = false;

/// Why the option is switched off on this server, if it is. Locked when the
/// server is in real-money mode OR the wallet's Ark server is on real
/// Bitcoin (`network` from ark-info): the two can disagree when a wallet
/// made by hand was adopted by the wizard, and the lock must follow the coins,
/// not the label. An unreadable network only leaves the mode to decide.
pub fn not_enabled_reason(mode: &str, wallet_network: Option<&str>) -> Option<&'static str> {
    let wallet_real = wallet_network
        .map(crate::wallet::barkd::is_mainnet_network)
        .unwrap_or(false);
    ((mode == "live" || wallet_real) && !ENABLED_ON_MAINNET).then_some(
        "The emergency withdrawal is not switched on for real money yet: it is being tested with play money first. Until then, keep only small amounts on this server.",
    )
}

/// Below this many sats on the server an emergency withdrawal is refused.
pub const MIN_EXIT_SATS: i64 = 100_000;
/// The Ark balance must be at least this many times the projected fees.
pub const MIN_FEE_MULTIPLE: i64 = 2;

/// Settings key: unix time the owner started an exit. Renewals and sweeps
/// pause while it is set (they would fight over the same coins).
pub const EXIT_STARTED_AT_KEY: &str = "exit_started_at";
/// Settings key: "true" while barkd reports outputs ready to claim.
pub const EXIT_CLAIMABLE_KEY: &str = "exit_claimable";

/// The fee estimate from `GET /exits/fee` (pinned OpenAPI
/// `EmergencyExitFeeEstimateResponse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExitFee {
    /// CPFP fees to broadcast the exit transactions, including barkd's fee
    /// margin. Paid from the server's confirmed on-chain balance: the amount
    /// the owner must deposit first.
    pub broadcast_sat: i64,
    /// The final claim transaction's fee, paid later out of the exited coins.
    pub claim_sat: i64,
    /// Both together: what the money-is-worth-it rule compares against.
    pub total_sat: i64,
    /// How many exit transactions still need broadcasting.
    pub txs_to_broadcast: i64,
}

/// Read the fee estimate. `None` when the answer lacks the fee fields.
pub fn parse_fee(v: &Json) -> Option<ExitFee> {
    let int = |k: &str| v.get(k).and_then(Json::as_i64).filter(|n| *n >= 0);
    let broadcast_sat = int("exit_broadcast_fee_sat")?;
    let claim_sat = int("claim_fee_sat")?;
    let total_sat = int("total_fee_sat").unwrap_or(broadcast_sat + claim_sat);
    Some(ExitFee {
        broadcast_sat,
        claim_sat,
        total_sat: total_sat.max(broadcast_sat + claim_sat),
        txs_to_broadcast: int("txs_to_broadcast").unwrap_or(0),
    })
}

/// Why an exit must not start now, in words for the owner; `None` = allowed.
/// Checked on the server when the exit is started, with a fresh estimate.
pub fn refusal(ark_sats: i64, fee: Option<&ExitFee>) -> Option<String> {
    let Some(fee) = fee else {
        return Some("Could not estimate the fees right now; try again later.".into());
    };
    let enough_money = ark_sats >= MIN_EXIT_SATS;
    let worth_the_fees = ark_sats >= MIN_FEE_MULTIPLE.saturating_mul(fee.total_sat);
    if enough_money && worth_the_fees {
        return None;
    }
    if !worth_the_fees {
        return Some(format!(
            "Not worth it right now. The network fees to take this money out on your own (about {} sats) are too high compared with what is on the server ({} sats). It is more economical to leave it, or to wait and hope the Ark server comes back online and cooperates. Fees change: check again later.",
            fee.total_sat, ark_sats
        ));
    }
    Some(format!(
        "Not worth it right now. There is less than {MIN_EXIT_SATS} sats on the server ({ark_sats} sats), and taking it out on your own costs about {} sats in network fees and a day or more. It is more economical to leave it, or to wait and hope the Ark server comes back online and cooperates.",
        fee.total_sat
    ))
}

/// How many more confirmed on-chain sats the server needs before the exit
/// fees are covered (0 = enough).
pub fn deposit_still_needed(fee: &ExitFee, confirmed_onchain_sats: i64) -> i64 {
    (fee.broadcast_sat - confirmed_onchain_sats.max(0)).max(0)
}

/// The confirmed-only on-chain balance (`confirmed_sat`). Exit fees can only
/// be paid from confirmed coins, so the "trusted pending" part does not count.
pub fn confirmed_onchain_sats(v: &Json) -> Option<i64> {
    v.get("confirmed_sat").and_then(Json::as_i64)
}

/// Unconfirmed sats on their way (shown as "waiting for a confirmation").
pub fn pending_onchain_sats(v: &Json) -> i64 {
    ["trusted_pending_sat", "untrusted_pending_sat"]
        .iter()
        .filter_map(|k| v.get(*k).and_then(Json::as_i64))
        .sum()
}

/// One coin's exit as `GET /exits/status/live` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveExit {
    pub vtxo_id: String,
    /// `start`, `processing`, `awaiting-delta`, `claimable`,
    /// `claim-in-progress`, ... (pinned `ExitState` `type`).
    pub state: String,
}

pub fn parse_live(v: &Json) -> Vec<LiveExit> {
    let Some(rows) = v.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            Some(LiveExit {
                vtxo_id: row.get("vtxo_id")?.as_str()?.to_string(),
                state: row
                    .get("state")
                    .and_then(|s| s.get("type"))
                    .and_then(Json::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
            })
        })
        .collect()
}

pub fn any_claimable(live: &[LiveExit]) -> bool {
    live.iter().any(|e| e.state == "claimable")
}

/// Plain words for an exit state.
pub fn state_words(state: &str) -> &'static str {
    match state {
        "start" => "starting",
        "processing" => "sending its transactions to the Bitcoin network",
        "awaiting-delta" => "waiting out the safety delay (about a day on mainnet)",
        "claimable" => "ready to claim",
        "claim-in-progress" => "claim sent, waiting for it to confirm",
        "claimed" => "claimed",
        "vtxo-already-spent" => "already spent (nothing to take out)",
        "vtxo-swept" => "expired and taken back by the Ark server",
        "canceled" => "stopped",
        _ => "in progress",
    }
}

/// The emergency withdrawal must start before the earliest coin expires,
/// minus the exit delta. `expiries` are unix times; `exit_delta_blocks`
/// from the server's terms (`vtxo_exit_delta`).
pub fn start_deadline(expiries: &[i64], exit_delta_blocks: Option<i64>) -> Option<i64> {
    let earliest = *expiries.iter().min()?;
    Some(earliest - exit_delta_blocks.unwrap_or(0).max(0) * 600)
}

/// The claim destination: an on-chain address for this server's network,
/// checked like a payout address. Never an Ark address (an exit leaves Ark).
pub fn check_destination(input: &str, mode: &str) -> Result<String, String> {
    let address = crate::payout::check(input, mode)?;
    if crate::config::is_ark_destination(&address) {
        return Err("an emergency withdrawal goes to a regular Bitcoin address (on-chain), not an Ark address: the point is to leave Ark".into());
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fee(total: i64) -> ExitFee {
        ExitFee {
            broadcast_sat: total - 100.min(total),
            claim_sat: 100.min(total),
            total_sat: total,
            txs_to_broadcast: 3,
        }
    }

    #[test]
    fn refused_below_100k_or_below_twice_the_fees() {
        // Owner rule, 12 Nov 2026.
        let r = refusal(99_999, Some(&fee(10))).unwrap();
        assert!(r.contains("less than 100000 sats"), "{r}");
        let r = refusal(200_000, Some(&fee(100_001))).unwrap();
        assert!(r.contains("too high compared with"), "{r}");
        assert!(
            r.contains("about 100001 sats") && r.contains("(200000 sats)"),
            "{r}"
        );
        assert_eq!(refusal(200_000, Some(&fee(100_000))), None);
        assert_eq!(refusal(100_000, Some(&fee(50_000))), None);
        assert!(refusal(100_000, Some(&fee(50_001))).is_some());
        // No estimate: refused.
        let r = refusal(10_000_000, None).unwrap();
        assert!(r.contains("Could not estimate"), "{r}");
    }

    #[test]
    fn the_fee_estimate_is_read_from_barkds_answer() {
        let v: Json = serde_json::from_str(
            r#"{"exit_broadcast_fee_sat": 4200, "claim_fee_sat": 800, "total_fee_sat": 5000,
                "fee_rate_sat_per_vb": 3.5, "txs_to_broadcast": 4, "fundable": false}"#,
        )
        .unwrap();
        assert_eq!(
            parse_fee(&v),
            Some(ExitFee {
                broadcast_sat: 4200,
                claim_sat: 800,
                total_sat: 5000,
                txs_to_broadcast: 4
            })
        );
        // A total smaller than its parts is never trusted.
        let odd: Json = serde_json::from_str(
            r#"{"exit_broadcast_fee_sat": 4200, "claim_fee_sat": 800, "total_fee_sat": 10}"#,
        )
        .unwrap();
        assert_eq!(parse_fee(&odd).unwrap().total_sat, 5000);
        assert_eq!(parse_fee(&serde_json::json!({"x": 1})), None);
        assert_eq!(
            parse_fee(&serde_json::json!({"exit_broadcast_fee_sat": -1, "claim_fee_sat": 1})),
            None
        );
    }

    #[test]
    fn the_deposit_covers_the_broadcast_fees_from_confirmed_coins_only() {
        let f = ExitFee {
            broadcast_sat: 4200,
            claim_sat: 800,
            total_sat: 5000,
            txs_to_broadcast: 4,
        };
        assert_eq!(deposit_still_needed(&f, 0), 4200);
        assert_eq!(deposit_still_needed(&f, 4000), 200);
        assert_eq!(deposit_still_needed(&f, 9000), 0);
        let bal: Json = serde_json::from_str(
            r#"{"total_sat": 6000, "trusted_spendable_sat": 6000, "immature_sat": 0,
                "trusted_pending_sat": 2000, "untrusted_pending_sat": 500, "confirmed_sat": 4000}"#,
        )
        .unwrap();
        assert_eq!(confirmed_onchain_sats(&bal), Some(4000));
        assert_eq!(pending_onchain_sats(&bal), 2500);
    }

    #[test]
    fn live_exits_are_listed_with_their_state() {
        let v: Json = serde_json::from_str(
            r#"[{"vtxo_id": "a:0", "state": {"type": "processing", "tip_height": 5}},
                {"vtxo_id": "b:1", "state": {"type": "claimable"}},
                {"state": {"type": "start"}}]"#,
        )
        .unwrap();
        let live = parse_live(&v);
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].state, "processing");
        assert!(any_claimable(&live));
        assert!(!any_claimable(&live[..1]));
        assert!(state_words("awaiting-delta").contains("delay"));
    }

    #[test]
    fn real_money_waits_for_the_signet_drill() {
        assert!(not_enabled_reason("test", None).is_none());
        assert!(not_enabled_reason("test", Some("signet")).is_none());
        assert_eq!(
            not_enabled_reason("live", None).is_some(),
            !ENABLED_ON_MAINNET
        );
        assert_eq!(
            not_enabled_reason("live", Some("signet")).is_some(),
            !ENABLED_ON_MAINNET
        );
        // A real-money wallet under a play-money label is still real money.
        assert_eq!(
            not_enabled_reason("test", Some("bitcoin")).is_some(),
            !ENABLED_ON_MAINNET
        );
    }

    #[test]
    fn the_deadline_is_the_earliest_expiry_minus_the_exit_delta() {
        assert_eq!(
            start_deadline(&[2_000_000, 1_000_000], Some(144)),
            Some(1_000_000 - 86_400)
        );
        assert_eq!(start_deadline(&[], Some(144)), None);
        assert_eq!(start_deadline(&[500], None), Some(500));
    }

    #[test]
    fn the_destination_is_on_chain_for_this_network() {
        let tb = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        assert_eq!(check_destination(tb, "test").unwrap(), tb);
        let e = check_destination(crate::payout::tests::TARK_A, "test").unwrap_err();
        assert!(e.contains("not an Ark address"), "{e}");
        assert!(check_destination("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4", "test").is_err());
        assert!(check_destination("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2", "live").is_ok());
    }
}
