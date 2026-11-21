//! Emergency withdrawal without the Ark server (a unilateral exit).
//!
//! The owner always holds the keys to the server's Ark coins, so they can be
//! taken out on-chain even if the Ark server is gone or refuses to help. It
//! is slow (several on-chain transactions per coin, then a wait of at least
//! the exit delta, about a day on mainnet) and costly (each transaction pays
//! network fees, from the server's own CONFIRMED on-chain balance). So it is
//! a last resort: nothing here ever starts on its own, and the server refuses
//! an exit that is not worth it (owner rule, 12 November 2026: at least
//! 100,000 sats AND at least twice the projected fees; the floor was lowered
//! to 50,000 sats on 16 November 2026 after a real exit cost about 700 sats).
//!
//! Pure rules live here; the barkd calls are in `wallet::barkd` and the HTTP
//! handlers in `routes::exit`.

use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value as Json;

use crate::error::AppError;

/// On for real money since 15 November 2026, at the owner's request, after
/// small live mainnet drills (exit started, broadcast, confirmed, claim
/// scheduled; see docs/dev/EMERGENCY_EXIT_2026-11-15.md). Set to `false` to
/// switch it off again on real money; the page then says so.
pub const ENABLED_ON_MAINNET: bool = true;

/// Why the option is switched off on this server, if it is. Locked when the
/// server is in real-money mode OR the wallet's Ark server is on real
/// Bitcoin (`network` from ark-info): the two can disagree when a wallet
/// made by hand was adopted by the wizard, and the lock must follow the coins,
/// not the label. An unreadable network only leaves the mode to decide.
pub fn not_enabled_reason(mode: &str, wallet_network: Option<&str>) -> Option<&'static str> {
    if ENABLED_ON_MAINNET {
        return None;
    }
    let wallet_real = wallet_network
        .map(crate::wallet::barkd::is_mainnet_network)
        .unwrap_or(false);
    (mode == "live" || wallet_real).then_some(
        "The emergency withdrawal is not switched on for real money yet: it is being tested with play money first. Until then, keep only small amounts on this server.",
    )
}

/// Fee money worth setting aside ahead of time (regular Bitcoin on the
/// server). Drill series 2: an exit needed about 350 sats of confirmed
/// on-chain money to broadcast; this leaves room for fee swings.
pub const RESERVE_MIN_SATS: i64 = 1_500;
/// What to recommend: room for the fees to rise before it is needed.
pub const RESERVE_RECOMMENDED_SATS: i64 = 5_000;
pub const RESERVE_RECOMMENDED_HIGH_SATS: i64 = 10_000;
const _: () = assert!(RESERVE_MIN_SATS < RESERVE_RECOMMENDED_SATS);
const _: () = assert!(RESERVE_RECOMMENDED_SATS < RESERVE_RECOMMENDED_HIGH_SATS);

/// The low end of the recommendation: the default, or twice the current
/// broadcast-fee estimate when that is higher.
pub fn reserve_target(fee: Option<&ExitFee>) -> i64 {
    fee.map_or(RESERVE_RECOMMENDED_SATS, |f| {
        RESERVE_RECOMMENDED_SATS.max(f.broadcast_sat.saturating_mul(2))
    })
}

/// The high end of the recommendation (twice the low end, at least 10,000).
pub fn reserve_high(target: i64) -> i64 {
    target.saturating_mul(2).max(RESERVE_RECOMMENDED_HIGH_SATS)
}

/// Settings key: "true" once the owner has turned the emergency withdrawal on
/// (it is opt-in, off until then). While it is on, the server keeps at least
/// [`RESERVE_FLOOR_SATS`] of regular Bitcoin for the fees and never sends it
/// out. An emergency withdrawal always takes out everything on the server.
pub const OPT_IN_KEY: &str = "exit_opt_in";
/// While the emergency withdrawal is on, at least this much regular Bitcoin
/// stays on the server to pay its network fees: nothing here sends it out, and
/// the page asks for more until it is there. Turn the emergency withdrawal off
/// and the money can be taken back.
pub const RESERVE_FLOOR_SATS: i64 = 5_000;
const _: () = assert!(RESERVE_FLOOR_SATS >= RESERVE_MIN_SATS);

/// Has the owner turned the emergency withdrawal on?
pub fn opted_in(conn: &Connection) -> Result<bool, AppError> {
    Ok(crate::settings::get(conn, OPT_IN_KEY)?.as_deref() == Some("true"))
}

/// Turn it on or off (the caller has already checked the password and codes).
pub fn set_opted_in(conn: &Connection, on: bool) -> Result<(), AppError> {
    if on {
        crate::settings::set(conn, OPT_IN_KEY, "true")
    } else {
        conn.execute("DELETE FROM settings WHERE key = ?1", [OPT_IN_KEY])?;
        Ok(())
    }
}

/// How much transaction space the "send back" estimate allows for (many small
/// inputs and two outputs, generously), times the network fee rate.
pub const LEFTOVER_FEE_VBYTES: i64 = 600;
/// The estimate is padded by half again, so a fee that comes out higher than
/// expected cannot dent the 5,000-sat floor the emergency withdrawal needs.
pub const LEFTOVER_FEE_PAD_PERCENT: i64 = 150;
/// Less than this above the floor is not worth a transaction.
pub const MIN_LEFTOVER_SEND_SATS: i64 = 1_000;

/// What the "send the fee money back" button sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leftover {
    /// The emergency withdrawal is off (or one has run): everything goes.
    Everything,
    /// It is on: only what is above the 5,000-sat floor, after the network fee.
    Amount(i64),
}

/// Decide what the button sends. While the emergency withdrawal is turned on
/// (and none has started), at least [`RESERVE_FLOOR_SATS`] stays: the amount is
/// the confirmed balance less the floor less an estimate of the network fee
/// (`fee_rate` sat/vB). Too little left is a plain refusal.
pub fn leftover_plan(
    opted_in: bool,
    exit_started: bool,
    confirmed_sats: i64,
    fee_rate_sat_vb: i64,
) -> Result<Leftover, String> {
    if !opted_in || exit_started {
        return Ok(Leftover::Everything);
    }
    let fee = fee_rate_sat_vb
        .max(1)
        .saturating_mul(LEFTOVER_FEE_VBYTES)
        .saturating_mul(LEFTOVER_FEE_PAD_PERCENT)
        / 100;
    let amount = confirmed_sats
        .max(0)
        .saturating_sub(RESERVE_FLOOR_SATS)
        .saturating_sub(fee);
    if amount < MIN_LEFTOVER_SEND_SATS {
        return Err(format!(
            "The emergency withdrawal is turned on, so at least {} sats of this money stays on the server for its fees. There are {} confirmed sats here; after keeping {} and paying about {} in network fees, too little would be left to send. To take all of it back, turn the emergency withdrawal off first (in this same section).",
            crate::payout::commas(RESERVE_FLOOR_SATS),
            crate::payout::commas(confirmed_sats.max(0)),
            crate::payout::commas(RESERVE_FLOOR_SATS),
            crate::payout::commas(fee),
        ));
    }
    Ok(Leftover::Amount(amount))
}

/// How many more confirmed on-chain sats the reserve needs to reach the floor
/// (0 = enough). Shown while the emergency withdrawal is on.
pub fn reserve_short_sats(confirmed_onchain_sats: i64) -> i64 {
    (RESERVE_FLOOR_SATS - confirmed_onchain_sats.max(0)).max(0)
}

/// Below this many sats on the server an emergency withdrawal is refused.
pub const MIN_EXIT_SATS: i64 = 50_000;
/// The Ark balance must be at least this many times the projected fees.
pub const MIN_FEE_MULTIPLE: i64 = 2;

/// Settings key: unix time the owner started an exit. Renewals and sweeps
/// pause while it is set (they would fight over the same coins).
pub const EXIT_STARTED_AT_KEY: &str = "exit_started_at";
/// Settings key: the address typed when the withdrawal was started. Cleared
/// together with [`EXIT_STARTED_AT_KEY`] once the withdrawal is over.
pub const EXIT_DESTINATION_KEY: &str = "exit_destination";
/// Settings key: "true" while barkd reports outputs ready to claim.
pub const EXIT_CLAIMABLE_KEY: &str = "exit_claimable";

/// The fee estimate from `GET /exits/fee` (pinned OpenAPI
/// `EmergencyExitFeeEstimateResponse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExitFee {
    /// barkd's own verdict: can the wallet pay for the exit transactions
    /// right now? `Some(false)` is a hard stop (mainnet drill: an exit sat
    /// stuck for hours with no confirmed on-chain money, and the broadcast
    /// needed 352 sats although the fee estimate said 99). `None` = not said.
    pub fundable: Option<bool>,
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
        fundable: v.get("fundable").and_then(Json::as_bool),
        broadcast_sat,
        claim_sat,
        total_sat: total_sat.max(broadcast_sat + claim_sat),
        txs_to_broadcast: int("txs_to_broadcast").unwrap_or(0),
    })
}

/// True when barkd's fee error means "these coins cannot be priced", not a
/// passing glitch. Seen live on mainnet (drill F9) for coins received over
/// Lightning: `Non-Standard VTXO ... exit chain is not relayable: ... below
/// the dust limit`. Display only; it never lets an exit start.
pub fn fee_cannot_be_priced(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("non-standard vtxo")
        || e.contains("not relayable")
        || e.contains("below the dust limit")
}

/// Settings key: unix time the server last asked the wallet to refresh coins
/// so an emergency withdrawal can be priced.
pub const EXIT_PREPARE_AT_KEY: &str = "exit_prepare_at";
/// A refresh round on real money takes about an hour; do not ask again
/// before then.
pub const PREPARE_GAP_SECS: i64 = 50 * 60;
/// How long the page keeps saying "getting your coins ready" after a refresh
/// was asked for.
pub const PREPARE_SHOWN_SECS: i64 = 2 * 3600;

/// Seconds to wait before another refresh may be asked for; `None` = go.
pub fn prepare_wait_secs(last_at: Option<i64>, now: i64) -> Option<i64> {
    let last = last_at?;
    let wait = last.saturating_add(PREPARE_GAP_SECS) - now;
    (wait > 0).then_some(wait)
}

/// [`refusal`], but when the fee estimate is missing because barkd cannot
/// price some coins, say that instead of the generic "try again later".
/// The answer is still a refusal either way.
pub fn refusal_explained(
    ark_sats: i64,
    fee: Option<&ExitFee>,
    fee_error: Option<&str>,
) -> Option<String> {
    if fee.is_none() && fee_error.is_some_and(fee_cannot_be_priced) {
        return Some(
            "The wallet cannot work out the fee yet for some of your coins. This happens with coins received over Lightning. This page asks the server to refresh them automatically, which takes up to an hour on real money, and it updates by itself. An emergency withdrawal stays blocked until the fee can be worked out, so nothing is at risk.".into(),
        );
    }
    refusal(ark_sats, fee)
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
///
/// The fee estimate alone is not enough: the exit transactions are paid for
/// through a child transaction that needed 352 sats when the estimate said
/// 99. So the floor is [`RESERVE_MIN_SATS`].
pub fn deposit_still_needed(fee: &ExitFee, confirmed_onchain_sats: i64) -> i64 {
    (fee.broadcast_sat.max(RESERVE_MIN_SATS) - confirmed_onchain_sats.max(0)).max(0)
}

/// A reason an exit must not start although the amounts look fine: the wallet
/// says it cannot fund the exit (`fundable: false`).
pub fn start_blocker(fee: &ExitFee) -> Option<String> {
    (fee.fundable == Some(false)).then(|| {
        "The wallet says it cannot pay for this withdrawal yet: it needs confirmed fee money (regular Bitcoin) on the server, and does not see enough. If you just sent some, wait for 1 confirmation and try again.".to_string()
    })
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

    #[test]
    fn the_emergency_withdrawal_is_opt_in_and_guards_its_fee_money() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        assert!(!opted_in(&c).unwrap(), "off until the owner turns it on");
        set_opted_in(&c, true).unwrap();
        assert!(opted_in(&c).unwrap());
        set_opted_in(&c, false).unwrap();
        assert!(!opted_in(&c).unwrap());
        // Off: the fee money can be sent out. On: it cannot, unless a
        // withdrawal is running (the running rules apply then).
        // Off, or a withdrawal has run: everything goes back.
        assert_eq!(
            leftover_plan(false, false, 50_000, 10),
            Ok(Leftover::Everything)
        );
        assert_eq!(
            leftover_plan(false, true, 50_000, 10),
            Ok(Leftover::Everything)
        );
        assert_eq!(
            leftover_plan(true, true, 50_000, 10),
            Ok(Leftover::Everything)
        );
        // On: only what is above 5,000, after the padded fee
        // (10 sat/vB x 600 vB x 1.5 = 9,000).
        assert_eq!(
            leftover_plan(true, false, 50_000, 10),
            Ok(Leftover::Amount(36_000))
        );
        assert_eq!(
            leftover_plan(true, false, 10_000, 1),
            Ok(Leftover::Amount(4_100))
        );
        // The floor holds even if the real fee is as much as the padded
        // estimate: what is left is never under 5,000.
        for confirmed in [6_000, 9_999, 12_345, 50_000, 1_000_000] {
            for rate in [1, 7, 25, 120] {
                if let Ok(Leftover::Amount(sent)) = leftover_plan(true, false, confirmed, rate) {
                    let padded_fee = rate * LEFTOVER_FEE_VBYTES * LEFTOVER_FEE_PAD_PERCENT / 100;
                    assert!(confirmed - sent - padded_fee >= RESERVE_FLOOR_SATS);
                    assert!(confirmed - sent >= RESERVE_FLOOR_SATS + padded_fee);
                }
            }
        }
        // Too little above the floor: refused, in plain words.
        let why = leftover_plan(true, false, 9_000, 10).expect_err("too little");
        assert!(why.contains("5,000"), "{why}");
        assert!(why.contains("9,000"), "{why}");
        assert!(why.contains("turn the emergency withdrawal off"), "{why}");
        assert!(leftover_plan(true, false, 0, 10).is_err());
        assert!(leftover_plan(true, false, -5, 10).is_err());
        // A fee rate of 0 is treated as 1 sat/vB, never as free.
        assert_eq!(
            leftover_plan(true, false, 6_000, 0),
            Err(leftover_plan(true, false, 6_000, 1).unwrap_err())
        );
        assert_eq!(RESERVE_FLOOR_SATS, 5_000);
        assert_eq!(reserve_short_sats(0), 5_000);
        assert_eq!(reserve_short_sats(-3), 5_000);
        assert_eq!(reserve_short_sats(4_999), 1);
        assert_eq!(reserve_short_sats(5_000), 0);
        assert_eq!(reserve_short_sats(9_000), 0);
    }

    fn fee(total: i64) -> ExitFee {
        ExitFee {
            fundable: None,
            broadcast_sat: total - 100.min(total),
            claim_sat: 100.min(total),
            total_sat: total,
            txs_to_broadcast: 3,
        }
    }

    #[test]
    fn refused_below_50k_or_below_twice_the_fees() {
        // Owner rule, 12 Nov 2026; floor lowered from 100,000 to 50,000 on 16 Nov 2026.
        let r = refusal(49_999, Some(&fee(10))).unwrap();
        assert!(r.contains("less than 50000 sats"), "{r}");
        assert_eq!(refusal(50_000, Some(&fee(10))), None);
        assert_eq!(
            refusal(99_999, Some(&fee(10))),
            None,
            "the old 100,000 floor no longer applies"
        );
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
    fn the_suggested_fee_reserve_follows_the_estimate() {
        assert_eq!(reserve_target(None), RESERVE_RECOMMENDED_SATS);
        assert_eq!(reserve_target(Some(&fee(500))), RESERVE_RECOMMENDED_SATS);
        // A busy fee market raises the suggestion.
        assert!(reserve_target(Some(&fee(5_000))) > RESERVE_RECOMMENDED_SATS);
        assert_eq!(reserve_high(5_000), 10_000);
        assert_eq!(reserve_high(12_000), 24_000);
    }

    #[test]
    fn a_refresh_is_not_asked_for_twice_within_an_hour() {
        assert_eq!(prepare_wait_secs(None, 1_000), None);
        assert_eq!(
            prepare_wait_secs(Some(1_000), 1_000 + 60),
            Some(PREPARE_GAP_SECS - 60)
        );
        assert_eq!(
            prepare_wait_secs(Some(1_000), 1_000 + PREPARE_GAP_SECS),
            None
        );
        assert_eq!(prepare_wait_secs(Some(1_000), 1_000 + 99_999), None);
    }

    #[test]
    fn unpriceable_coins_get_their_own_message() {
        let live = "barkd returned HTTP 500: Failed to estimate emergency exit fee: Non-Standard VTXO x: exit chain is not relayable: the VTXO's own output is below the dust limit for its script type";
        assert!(fee_cannot_be_priced(live));
        assert!(!fee_cannot_be_priced("connection failed"));
        let r = refusal_explained(10_000_000, None, Some(live)).unwrap();
        assert!(r.contains("cannot work out the fee yet"), "{r}");
        assert!(r.contains("nothing is at risk"), "{r}");
        // Other failures keep the generic wording; still refused.
        let r = refusal_explained(10_000_000, None, Some("connection failed")).unwrap();
        assert!(r.contains("Could not estimate"), "{r}");
        let r = refusal_explained(10_000_000, None, None).unwrap();
        assert!(r.contains("Could not estimate"), "{r}");
        // A good estimate is judged exactly as before.
        assert_eq!(
            refusal_explained(200_000, Some(&fee(100_000)), None),
            refusal(200_000, Some(&fee(100_000)))
        );
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
                fundable: Some(false),
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
            fundable: None,
            broadcast_sat: 4200,
            claim_sat: 800,
            total_sat: 5000,
            txs_to_broadcast: 4,
        };
        assert_eq!(deposit_still_needed(&f, 0), 4200);
        assert_eq!(deposit_still_needed(&f, 4000), 200);
        assert_eq!(deposit_still_needed(&f, 9000), 0);
        // A tiny estimate still asks for the floor (drill: estimate 99, needed 352).
        let tiny = ExitFee {
            broadcast_sat: 99,
            claim_sat: 17,
            total_sat: 116,
            ..f
        };
        assert_eq!(deposit_still_needed(&tiny, 0), RESERVE_MIN_SATS);
        assert_eq!(deposit_still_needed(&tiny, 352), RESERVE_MIN_SATS - 352);
        assert_eq!(deposit_still_needed(&tiny, RESERVE_MIN_SATS), 0);
        // The wallet's own "cannot fund this" is a hard stop.
        assert!(start_blocker(&ExitFee {
            fundable: Some(false),
            ..tiny
        })
        .is_some());
        assert!(start_blocker(&ExitFee {
            fundable: Some(true),
            ..tiny
        })
        .is_none());
        assert!(start_blocker(&ExitFee {
            fundable: None,
            ..tiny
        })
        .is_none());
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
    fn the_real_money_switch_follows_the_flag() {
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
