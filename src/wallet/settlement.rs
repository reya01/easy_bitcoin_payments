//! Settlement engine (Phase 1, Custody Level 0).
//!
//! Two jobs, in priority order:
//!
//! 1. **Renewal** — refresh spendable VTXOs inside the renewal window,
//!    before their per-VTXO expiry (learn_lifetime.md). Renewal never moves
//!    funds.
//! 2. **Sweep** — once the till holds the send minimum (`payout::sweep_min`), move
//!    all of it to the merchant's own wallet: an Ark send when the destination is on the
//!    same ASP (e.g. their Arké wallet), or a cooperative offboard of the
//!    whole till when it is an on-chain address (learn_offboard.md).
//!    Emergency exits are never automated: the owner starts one from
//!    Settings (`routes::exit`), and while it runs this engine pauses.
//!
//! An automatic ON-CHAIN sweep also waits until the network fee rate is at or
//! below `onchain_sweep_max_fee_sat_vb`; the merchant can always sweep by hand.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use crate::config::Config;
use crate::db::Db;
use crate::error::AppError;
use crate::util::{new_id, now_unix};
use crate::wallet::barkd::{vtxo_expiry_unix, BarkdClient};

/// What one settlement tick did, for logs and tests.
#[derive(Debug, Default)]
pub struct TickReport {
    pub refreshed_vtxos: usize,
    pub swept_sats: Option<i64>,
    pub sweep_refused: Option<String>,
    /// Money that is the shop's but could not be part of this send, and why.
    /// "Move my money now" shows it so the owner is never left wondering why
    /// not everything moved.
    pub held_back: Vec<HeldBack>,
}

/// One part of the till that cannot be sent yet. `kind` is one of
/// `renewing` (locked in a coin renewal round), `collecting` (a Lightning
/// payment still being collected) or `confirming` (waiting for confirmations).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBack {
    pub kind: &'static str,
    pub sats: i64,
}

/// What cannot be sent right now, from the coins locked in a renewal round
/// (`locked_sats`, which the wallet may still count as spendable) and the
/// wallet's own "on its way" figures. Zero amounts are left out. A renewal
/// is counted once: the larger of the two figures, because the wallet may or
/// may not already count the locked coins in its round figure.
pub(crate) fn held_back_parts(
    locked_sats: i64,
    b: &crate::wallet::barkd::Balances,
) -> Vec<HeldBack> {
    let mut out = Vec::new();
    let renewing = locked_sats.max(b.pending_in_round_sats);
    for (kind, sats) in [
        ("renewing", renewing),
        ("collecting", b.claimable_lightning_sats),
        ("confirming", b.pending_board_sats),
    ] {
        if sats > 0 {
            out.push(HeldBack { kind, sats });
        }
    }
    out
}

/// Bitcoin's target block time, used to turn blocks into seconds.
const SECS_PER_BLOCK: i64 = 600;

/// Never wait until fewer blocks than this are left (about two hours): room
/// for a refresh to be retried.
/// An automatic sweep only goes ahead when at least this many sats are free.
/// Below it, the money stays and the next payment
/// adds to it. 1,500 (it was 1,000) so the coin that arrives at the payout
/// wallet is not a small one, see [`SMALL_COIN_SATS`].
pub const MIN_SWEEP_EXCESS_SATS: i64 = 1_500;

/// The floor, and the default, for the smallest automatic sweep to an on-chain
/// address (the owner can choose more in Settings, see `payout::sweep_min`): every on-chain withdrawal pays a network fee that can
/// be high and changes a lot, so fewer, bigger withdrawals cost much less.
/// (Ark sweeps are almost free and use [`MIN_SWEEP_EXCESS_SATS`].) "Move my
/// money now" ignores it.
pub const MIN_ONCHAIN_SWEEP_EXCESS_SATS: i64 = 50_000;

/// Most coins in one refresh request. Bark 0.9 takes the 1,000 soonest-expiring
/// coins and its server refuses larger refreshes from older wallets, so a
/// request never asks for more; the rest follow on the next cycle.
pub const MAX_REFRESH_COINS: usize = 1_000;

/// Bark 0.9's changelog: a payment that spends a coin under "about 1,300 sats"
/// can fail with an upgrade error when the wallet is Bark 0.8.1 or older and
/// the Ark server is 0.9 (see docs/dev/archive/BARK_0_9_REVIEW.md). Shown on Settings
/// as guidance next to the send minimum; nothing is forced. The figure is from
/// the changelog and is to be confirmed by Second.
pub const SMALL_COIN_SATS: i64 = 1_300;

/// The smallest Ark payment EBP will make. Bark 0.9 sends any output under
/// 330 sats to a separate "dust isolation" transaction (see
/// docs/dev/archive/BARK_0_9_REVIEW.md), so a send this small is held back instead.
/// Automatic sweeps are always far above it (their floor is 1,500); only
/// "Move my money now" can get here.
pub const ARK_MIN_SEND_SATS: i64 = 330;

/// What an on-chain withdrawal is assumed to weigh when the network fee is
/// estimated (virtual bytes). Deliberately on the high side.
pub const ONCHAIN_SEND_VBYTES_ESTIMATE: i64 = 400;

/// An on-chain withdrawal is held back unless at least this many sats would be
/// left for the owner after the estimated network fee: well above Bitcoin's
/// own dust limit (546) and Bark's isolation size (330), so what arrives is a
/// normal coin.
pub const ONCHAIN_MIN_ARRIVAL_SATS: i64 = 1_000;

/// The fee rate (sat/vB) assumed for the check above when the real one cannot
/// be read. Only a button press gets here (an automatic withdrawal that cannot
/// read fees waits, as before).
pub const UNREADABLE_FEE_GUESS_SAT_VB: i64 = 50;

/// `Some(plain words)` when an on-chain withdrawal of `total` sats would leave
/// too little after the estimated network fee at `fee_sat_vb`. Nothing is sent
/// then: the money stays in the till.
pub(crate) fn onchain_arrival_problem(total: i64, fee_sat_vb: i64) -> Option<String> {
    let fee = fee_sat_vb
        .max(0)
        .saturating_mul(ONCHAIN_SEND_VBYTES_ESTIMATE);
    let arrives = total.saturating_sub(fee);
    if arrives >= ONCHAIN_MIN_ARRIVAL_SATS {
        return None;
    }
    let what = if arrives <= 0 {
        format!(
            "The network fee for an on-chain withdrawal (about {fee} sats at today's fees) is more than the {total} sats that would be sent."
        )
    } else {
        format!(
            "Only about {arrives} sats would arrive after the network fee (about {fee} sats at today's fees)."
        )
    };
    Some(format!(
        "{what} That is too little to be worth sending, so nothing was sent and your money stays in the till. It goes out once there is more, or when fees are lower."
    ))
}

/// Setting: unix time of the last send the Ark server (or barkd) refused
/// because the wallet software is too old (`"0"` or missing = no such refusal).
pub const WALLET_UPDATE_NEEDED_KEY: &str = "wallet_update_needed_at";
/// After such a refusal the automatic transfer stops asking for this long
/// (then tries once more, so it resumes by itself after an update). "Move my
/// money now" always tries.
const UPDATE_NEEDED_PAUSE_SECS: i64 = 6 * 3600;
/// Home shows the "needs an update" line for this long after the last refusal
/// (a retry that is refused again renews it), and the phone alert is repeated
/// no sooner than this.
pub const UPDATE_NEEDED_SHOW_SECS: i64 = 24 * 3600;

/// Does this error from barkd say the wallet software is too old for the Ark
/// server? Bark 0.9's changelog says an older wallet "can fail with an upgrade
/// error" when it spends a small coin (docs/dev/archive/BARK_0_9_REVIEW.md). The exact
/// text is not yet confirmed by Second, so this looks for the words that any
/// such message must carry. A wrong guess only pauses the automatic transfer
/// for a few hours and shows a line on Home; no money is ever moved by it.
pub(crate) fn is_update_needed_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    [
        "upgrade",
        "outdated",
        "too old",
        "update your wallet",
        "unsupported client version",
        "unsupported wallet version",
    ]
    .iter()
    .any(|w| m.contains(w))
}

fn update_needed_at(conn: &Connection) -> i64 {
    crate::settings::get(conn, WALLET_UPDATE_NEEDED_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
}

/// The automatic transfer should not ask again yet.
pub(crate) fn update_needed_paused(conn: &Connection, now: i64) -> bool {
    let at = update_needed_at(conn);
    at > 0 && now - at < UPDATE_NEEDED_PAUSE_SECS
}

/// Home should show the "needs an update" line.
pub fn update_needed_showing(conn: &Connection, now: i64) -> bool {
    let at = update_needed_at(conn);
    at > 0 && now - at < UPDATE_NEEDED_SHOW_SECS
}

/// Remember a refusal for old wallet software; queue ONE phone alert (not one
/// per retry).
fn mark_update_needed(conn: &Connection, now: i64) -> Result<(), AppError> {
    let before = update_needed_at(conn);
    crate::settings::set(conn, WALLET_UPDATE_NEEDED_KEY, &now.to_string())?;
    if before <= 0 || now - before >= UPDATE_NEEDED_SHOW_SECS {
        crate::alerts::queue_notice(conn, "wallet_update_needed")?;
    }
    Ok(())
}

const MIN_TRIGGER_BLOCKS: i64 = 12;

/// How many blocks before expiry a coin is refreshed.
///
/// Refreshing is cheaper the closer a coin is to expiry (on mainnet it is
/// free in the last two days and 0.2% before that), so we wait as long as is
/// safe: until the coin is inside the server's free window, and no earlier
/// than the configured lead time asks. A third of the coin lifetime is the
/// outer limit so a short-lived coin is never refreshed the moment it is
/// made (a new coin on a server whose coins live a day would otherwise be
/// "due" at once, forever). If the server's terms are unknown this is just
/// the configured lead time, the older and more cautious rule.
pub fn refresh_trigger_blocks(
    lead_secs: u64,
    terms: Option<crate::wallet::barkd::ArkTerms>,
) -> i64 {
    let mut blocks = ((lead_secs as i64) / SECS_PER_BLOCK).max(1);
    let Some(terms) = terms else {
        return blocks;
    };
    if let Some(free_below) = terms.free_refresh_below_blocks {
        blocks = blocks.min(free_below - 1);
    }
    if let Some(lifetime) = terms.vtxo_lifetime_blocks {
        blocks = blocks.min(lifetime / 3);
    }
    let mut blocks = blocks.max(MIN_TRIGGER_BLOCKS);
    // A refresh waits for the server's next round. If rounds are so slow
    // that fewer than MIN_ROUNDS_BEFORE_EXPIRY fit between the renewal point
    // and expiry, renew earlier, even if that costs a fee: not expiring
    // matters more. Never past half a coin's life, so a fresh coin is not
    // due again the moment it is made.
    if let Some(interval) = terms.round_interval_secs.filter(|s| *s > 0) {
        let mut needed =
            (MIN_ROUNDS_BEFORE_EXPIRY * interval + SECS_PER_BLOCK - 1) / SECS_PER_BLOCK;
        if let Some(lifetime) = terms.vtxo_lifetime_blocks {
            needed = needed.min(lifetime / 2);
        }
        blocks = blocks.max(needed);
    }
    blocks
}

/// Plain words for how long a coin has left.
fn time_left_words(secs: i64) -> String {
    if secs < 3_600 {
        "less than an hour".to_string()
    } else if secs < 2 * 86_400 {
        let h = secs / 3_600;
        format!("about {h} {}", if h == 1 { "hour" } else { "hours" })
    } else {
        format!("about {} days", secs / 86_400)
    }
}

/// Why a renewal was asked for, in plain words, for the note on the upkeep
/// line. Coins paid over Ark keep the payer's expiry, so a coin can arrive
/// already close to expiring; the wallet can also say coins must be
/// refreshed before they can be sent; and small coins can be added so the
/// request reaches the wallet's minimum.
pub(crate) fn renew_reason(
    timed: usize,
    soonest_secs: Option<i64>,
    flagged: usize,
    merged: usize,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if timed > 0 {
        parts.push(match soonest_secs {
            Some(s) => format!(
                "{timed} {} close to expiring (the soonest has {} left)",
                if timed == 1 { "coin is" } else { "coins are" },
                time_left_words(s.max(0))
            ),
            None => format!(
                "{timed} {} due and the expiry could not be read",
                if timed == 1 { "coin is" } else { "coins are" }
            ),
        });
    }
    if flagged > 0 {
        parts.push(format!(
            "your wallet said {flagged} more {} be refreshed before {} can be sent",
            if flagged == 1 {
                "coin must"
            } else {
                "coins must"
            },
            if flagged == 1 { "it" } else { "they" }
        ));
    }
    if merged > 0 {
        parts.push(format!(
            "{merged} small {} added so the total reaches the {MIN_RENEW_SATS}-sat minimum",
            if merged == 1 {
                "coin was"
            } else {
                "coins were"
            }
        ));
    }
    if parts.is_empty() {
        "the wallet asked for them".to_string()
    } else {
        parts.join("; ")
    }
}

/// At least this many rounds must fit between the renewal point and expiry
/// (room for a failed round to be retried).
const MIN_ROUNDS_BEFORE_EXPIRY: i64 = 4;

/// The smallest amount the wallet can renew: coins being refreshed must add
/// up to at least Bitcoin's taproot dust limit (pinned OpenAPI, refresh and
/// offboard requests: "must be >= P2TR_DUST"). The owner's signet test saw a
/// 136-sat renewal refused.
pub const MIN_RENEW_SATS: i64 = 330;

/// Settings keys for coins too small to renew (Home shows one plain line).
pub const TOO_SMALL_SATS_KEY: &str = "renewal_too_small_sats";
pub const TOO_SMALL_LAPSE_AT_KEY: &str = "renewal_too_small_lapse_at";
/// Settings key: renewals requested but not yet seen finished (JSON).
const REQUESTED_RENEWALS_KEY: &str = "renewals_requested";
/// A requested renewal whose coins are still there, with no round holding
/// them, is counted as failed after this long (and its coins become eligible
/// again). Long enough for an hourly mainnet round plus its confirmations, so
/// a round list we misread can never turn a working renewal into a false
/// "renewing is not working" warning.
const REQUESTED_GIVE_UP_SECS: i64 = 2 * 3600;

/// A coin this close to expiry (a quarter of the renewal lead time) is asked
/// for again even if a round is supposed to hold it, at most this often. A
/// round participation that never finishes must not be the reason a coin
/// expires: asking again costs nothing, expiring costs the coin.
const LAST_RESORT_RETRY_SECS: i64 = 3600;
/// A coin outside the server's free renewal window is only pulled into a
/// renewal to rescue a too-small coin if it is at most this many times the
/// rescued amount (its renewal fee, at most about 0.5%, then stays below
/// what is rescued). Matters with an on-chain payout, where the server may
/// hold one large coin.
const MERGE_MAX_PAID_MULTIPLE: i64 = 100;

/// An emergency withdrawal is running (see `exit_pause`).
struct ExitPause {
    reason: String,
    /// Coins being withdrawn; `None` when barkd could not say.
    exiting: Option<std::collections::HashSet<String>>,
}

/// A renewal the wallet accepted, waiting for its round.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RequestedRenewal {
    /// The `settlements` row that says "requested".
    pub row: String,
    /// The coins sent to the round.
    pub inputs: Vec<String>,
    /// When it was requested (unix seconds).
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RenewalOutcome {
    Waiting,
    Renewed,
    Failed(&'static str),
}

/// What happened to a requested renewal, from the coin list and the round
/// list. `present`: coin ids still in the wallet (spendable or locked).
/// `held` / `dead`: inputs of active / failed-or-canceled participations.
/// `rounds_known`: the round list could be read this cycle.
pub(crate) fn judge_requested(
    r: &RequestedRenewal,
    present: &std::collections::HashSet<&str>,
    held: &std::collections::HashSet<&str>,
    dead: &std::collections::HashSet<&str>,
    rounds_known: bool,
    now: i64,
) -> RenewalOutcome {
    let any =
        |set: &std::collections::HashSet<&str>| r.inputs.iter().any(|i| set.contains(i.as_str()));
    if any(held) {
        return RenewalOutcome::Waiting;
    }
    if any(dead) {
        return RenewalOutcome::Failed(
            "the Ark server's round did not complete; the coins will be renewed again",
        );
    }
    if !any(present) {
        // The old coins are gone: the round replaced them with fresh ones.
        return RenewalOutcome::Renewed;
    }
    if now - r.at >= REQUESTED_GIVE_UP_SECS {
        return RenewalOutcome::Failed(if rounds_known {
            "the coins were not taken into a round; they will be renewed again"
        } else {
            "no sign of the round finishing; the coins will be renewed again"
        });
    }
    RenewalOutcome::Waiting
}

/// barkd (or Second's server) refused because the coins are already in a
/// round: that is renewal in progress, not a failure. Matches the wording
/// loosely on purpose; anything else is still counted as a failure.
pub(crate) fn is_already_in_round(reason: &str) -> bool {
    let r = reason.to_lowercase();
    let about_coins = r.contains("round") || r.contains("participat") || r.contains("vtxo");
    about_coins
        && (r.contains("already")
            || r.contains("held")
            || r.contains("replace")
            || r.contains("locked"))
}

/// One settlement cycle at a time, whoever starts it (the timer or the
/// owner's "move my money now" button), so two cycles never renew or send
/// the same coins at once.
static CYCLE_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Minimum gap between refresh attempts for VTXOs whose expiry we cannot
/// determine (no timestamp and no resolvable height).
const UNKNOWN_EXPIRY_RETRY_SECS: i64 = 3600;

/// Pick VTXOs (largest first) until they cover `target` sats. Returns the
/// chosen ids and their total. Offboarding is per whole VTXO, so the total
/// can exceed `target` by up to one VTXO; it is never below it unless the
/// wallet holds less than `target` spendable.
pub(crate) fn select_vtxos_covering(
    vtxos: &[crate::wallet::barkd::Vtxo],
    target: i64,
) -> (Vec<String>, i64) {
    let mut sorted: Vec<&crate::wallet::barkd::Vtxo> =
        vtxos.iter().filter(|v| !v.id.is_empty()).collect();
    sorted.sort_by_key(|v| std::cmp::Reverse(v.amount_sats));
    let (mut ids, mut total) = (Vec::new(), 0i64);
    for v in sorted {
        if total >= target {
            break;
        }
        ids.push(v.id.clone());
        total += v.amount_sats;
    }
    (ids, total)
}

/// Settings key: the id of the unsettled transfer the phone alert was already
/// sent for, so a long wait sends one alert, not one per check.
/// A "sending" line is shown to the owner as unclear only after this long.
const SENDING_SHOWN_AFTER_SECS: i64 = 120;
const TRANSFER_UNKNOWN_ALERTED_KEY: &str = "transfer_unknown_alerted";
/// A send must be at least this much older than the history entry it is
/// matched with, in the other direction: history times can differ a little.
const HISTORY_MATCH_SLACK_SECS: i64 = 300;
/// How far above the amount a matching history entry may be (network fee).
const HISTORY_MATCH_FEE_ROOM_SATS: i64 = 10_000;
/// An on-chain send whose coins are all still in the wallet this long after
/// the attempt did not happen (a withdrawal waits for an Ark round, and the
/// wallet can list coins in a round as free for up to about an hour).
const OFFBOARD_NOT_SENT_AFTER_SECS: i64 = 7_200;

/// A send whose outcome nobody knows: the server wrote "sending" and then
/// restarted, or the request timed out before an answer came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsettledSend {
    pub id: String,
    pub kind: String,
    pub amount_sats: i64,
    pub destination: String,
    pub created_at: i64,
    pub vtxos: Vec<String>,
}

/// The oldest transfer whose outcome is unknown, if any. Sends run one at a
/// time under [`CYCLE_LOCK`], so a "sending" line found when a cycle starts
/// was left behind by a restart.
pub fn unsettled_send(conn: &Connection) -> Result<Option<UnsettledSend>, AppError> {
    unsettled_send_older_than(conn, 0)
}

/// [`unsettled_send`], but a "sending" line counts only once it is at least
/// `sending_min_age_secs` old. Home and the owner's two buttons use this, so a
/// transfer that is merely in flight right now is never shown as unclear.
pub fn unsettled_send_older_than(
    conn: &Connection,
    sending_min_age_secs: i64,
) -> Result<Option<UnsettledSend>, AppError> {
    let row = conn
        .query_row(
            "SELECT id, kind, amount_sats, destination, created_at, detail FROM settlements
             WHERE (status = 'unknown' OR (status = 'sending' AND created_at <= ?1))
             ORDER BY created_at ASC, rowid ASC LIMIT 1",
            params![now_unix() - sending_min_age_secs],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    Ok(
        row.map(|(id, kind, amount_sats, destination, created_at, detail)| {
            let vtxos = detail
                .as_deref()
                .and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
                .and_then(|v| v.get("vtxos").cloned())
                .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
                .unwrap_or_default();
            UnsettledSend {
                id,
                kind,
                amount_sats,
                destination,
                created_at,
                vtxos,
            }
        }),
    )
}

/// Home should show the "a transfer may or may not have gone out" line.
pub fn transfer_unknown_showing(conn: &Connection) -> bool {
    matches!(
        unsettled_send_older_than(conn, SENDING_SHOWN_AFTER_SECS),
        Ok(Some(_))
    )
}

/// The owner says how an unknown transfer ended. Returns false if there was
/// none to settle.
pub fn resolve_unsettled(conn: &Connection, arrived: bool) -> Result<bool, AppError> {
    let Some(u) = unsettled_send_older_than(conn, SENDING_SHOWN_AFTER_SECS)? else {
        return Ok(false);
    };
    let (status, detail) = if arrived {
        ("broadcast", "You confirmed it arrived.")
    } else {
        (
            "failed",
            "You confirmed it did not arrive; nothing was sent.",
        )
    };
    conn.execute(
        "UPDATE settlements SET status = ?1, detail = ?2 WHERE id = ?3",
        params![status, detail, u.id],
    )?;
    Ok(true)
}

/// Before the owner's "It did not arrive" is accepted: does the wallet's own
/// history show this transfer? `Ok(None)` = there is no unclear transfer,
/// `Ok(Some(true))` = the history shows it (so "did not arrive" would be wrong
/// and the money could be sent twice), `Ok(Some(false))` = it does not show.
/// An error means the history could not be read, which is NOT the same as
/// "not shown": the caller must refuse and ask to try again.
pub async fn unsettled_shows_in_history(
    client: &BarkdClient,
    db: &Db,
) -> Result<Option<bool>, AppError> {
    let Some(u) = db
        .call(|c| unsettled_send_older_than(c, SENDING_SHOWN_AFTER_SECS))
        .await?
    else {
        return Ok(None);
    };
    let page = client.history().await?;
    Ok(Some(history_may_show_send(&u, &page.entries)))
}

/// Could the wallet history show this send going out, or still going out?
/// The guard for the owner's "It did not arrive", so it is generous on
/// purpose: any movement to the same destination (or an outbound one whose
/// destinations cannot be read) that is not clearly older than the attempt,
/// whatever its amount, and whatever its status except a definite end
/// without sending (`failed`, `canceled`). A `pending` send is still on its
/// way: saying it did not arrive would let it be sent again.
pub(crate) fn history_may_show_send(
    u: &UnsettledSend,
    entries: &[crate::wallet::barkd::HistoryEntry],
) -> bool {
    entries.iter().any(|e| {
        let ended_unsent = e.status.as_deref().is_some_and(|s| {
            s.eq_ignore_ascii_case("failed") || s.eq_ignore_ascii_case("canceled")
        });
        let names_it = match e.sent_to_destinations() {
            Some(_) => e.sent_to_contains(&u.destination),
            None => e.outbound_sats().is_some(),
        };
        !ended_unsent
            && names_it
            && e.detected_at_unix()
                .is_none_or(|t| t >= u.created_at - HISTORY_MATCH_SLACK_SECS)
    })
}

/// Plain words for the sweep note and Home while a transfer is unclear.
pub const TRANSFER_UNKNOWN_NOTE: &str = "A transfer to your payout address may or may not have gone out: it was interrupted before the server heard back. Your money is safe either way. No more transfers are made until this is cleared, so it is not sent again by mistake. Check your payout wallet, then tell the server below whether it arrived.";

/// Did this failed send possibly go through anyway? A timeout, a dropped
/// connection after the request left, or an answer we could not read: yes.
/// A clear refusal, a connection that never opened or an error answer: no.
pub(crate) fn send_outcome_unknown(err: &crate::wallet::WalletError) -> bool {
    use crate::wallet::WalletError;
    match err {
        WalletError::Unavailable(m) => {
            m.contains("timed out")
                || m.contains("request failed")
                // A gateway that gave up waiting says nothing about whether
                // the send went through behind it.
                || m.contains("HTTP 502")
                || m.contains("HTTP 504")
        }
        WalletError::Rejected(m) => m.contains("unparseable response"),
    }
}

/// How many wallet-history entries look like this unclear send: outbound,
/// successful (or not yet reported either way), to the same destination, for
/// about the same amount, and not older than the attempt (minus a little
/// slack) nor in the future.
pub(crate) fn count_history_matches(
    u: &UnsettledSend,
    entries: &[crate::wallet::barkd::HistoryEntry],
    now: i64,
) -> usize {
    entries
        .iter()
        .filter(|e| {
            e.status
                .as_deref()
                .is_none_or(|s| s.eq_ignore_ascii_case("successful"))
                && e.outbound_sats().is_some_and(|n| {
                    n >= u.amount_sats && n <= u.amount_sats + HISTORY_MATCH_FEE_ROOM_SATS
                })
                && e.sent_to_contains(&u.destination)
                && e.detected_at_unix().is_some_and(|t| {
                    t >= u.created_at - HISTORY_MATCH_SLACK_SECS
                        && t <= now + HISTORY_MATCH_SLACK_SECS
                })
        })
        .count()
}

/// Could any wallet-history entry be this send, whatever its status or
/// amount? Used before concluding a send did NOT happen, so it counts
/// generously: any movement naming the destination (or with a `sent_to` we
/// cannot read) that is not clearly older than the attempt.
pub(crate) fn any_history_trace(
    u: &UnsettledSend,
    entries: &[crate::wallet::barkd::HistoryEntry],
) -> bool {
    entries.iter().any(|e| {
        let names_it = match e.sent_to_destinations() {
            Some(_) => e.sent_to_contains(&u.destination),
            // An outbound movement whose destinations we cannot read could be it.
            None => e.outbound_sats().is_some(),
        };
        names_it
            && e.detected_at_unix()
                .is_none_or(|t| t >= u.created_at - HISTORY_MATCH_SLACK_SECS)
    })
}

/// Do the wallet's round participations prove none of these coins is still
/// waiting in a round? `None` (the list could not be read) proves nothing,
/// and neither does an active participation whose coins we cannot read.
pub(crate) fn rounds_clear_of(
    coins: &[String],
    participations: Option<&[crate::wallet::barkd::RoundParticipation]>,
) -> bool {
    participations.is_some_and(|ps| {
        !ps.iter().any(|p| {
            p.is_active() && (p.inputs.is_empty() || p.inputs.iter().any(|i| coins.contains(i)))
        })
    })
}

pub struct SettlementEngine {
    client: BarkdClient,
    db: Db,
    config: Arc<Config>,
    /// Last time we refreshed unknown-expiry VTXOs (in-memory throttle).
    last_unknown_refresh: std::sync::Mutex<Option<i64>>,
    /// Last refused-sweep reason recorded, so a persistent refusal writes
    /// ONE `settlements` row instead of one per tick (was unbounded growth).
    last_refusal: std::sync::Mutex<Option<String>>,
}

impl SettlementEngine {
    pub fn new(client: BarkdClient, db: Db, config: Arc<Config>) -> Self {
        Self {
            client,
            db,
            config,
            last_unknown_refresh: std::sync::Mutex::new(None),
            last_refusal: std::sync::Mutex::new(None),
        }
    }

    fn forget_refusal(&self) {
        *self.last_refusal.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Run forever on the configured interval.
    pub async fn run(self) {
        let mut ticker = tokio::time::interval(Duration::from_secs(
            self.config.settlement_interval_secs.max(1),
        ));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // SUPERVISION (beta hardening): one spawned task per tick so a panic
        // is logged and the engine keeps running (renewal is what keeps
        // funds alive; it must not die silently).
        let this = Arc::new(self);
        loop {
            ticker.tick().await;
            let worker = Arc::clone(&this);
            match tokio::spawn(async move { worker.tick().await }).await {
                Ok(Ok(report))
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
                Ok(Ok(_)) => {}
                Ok(Err(err)) => tracing::warn!(error = %err, "settlement tick failed"),
                Err(join_err) => {
                    tracing::error!(error = %join_err, "settlement tick PANICKED; continuing")
                }
            }
        }
    }

    /// One settlement cycle.
    pub async fn tick(&self) -> Result<TickReport, AppError> {
        self.cycle(false).await
    }

    /// The merchant pressed "move my money now": one cycle that sweeps the
    /// WHOLE till to the payout address, ignoring the send minimum and the
    /// on-chain fee limit (they asked for it).
    pub async fn sweep_now(&self) -> Result<TickReport, AppError> {
        self.cycle(true).await
    }

    /// Bring "requested" renewals up to date (renewed, failed or still
    /// waiting) and return the coin ids that must not be asked for again
    /// this cycle: inputs of active round participations and of renewals
    /// still waiting.
    async fn settle_requested_renewals(
        &self,
        vtxos: &crate::wallet::barkd::VtxoSet,
        participations: Option<&[crate::wallet::barkd::RoundParticipation]>,
        now: i64,
    ) -> Result<
        (
            std::collections::HashSet<String>,
            std::collections::HashSet<String>,
        ),
        AppError,
    > {
        use std::collections::HashSet;
        let present: HashSet<String> = vtxos
            .spendable
            .iter()
            .chain(vtxos.pending.iter())
            .map(|v| v.id.clone())
            .collect();
        let mut held: HashSet<String> = HashSet::new();
        let mut dead: HashSet<String> = HashSet::new();
        for p in participations.unwrap_or(&[]) {
            let into = if p.is_active() { &mut held } else { &mut dead };
            into.extend(p.inputs.iter().cloned());
        }
        let rounds_known = participations.is_some();
        let (held_c, present_c, dead_c) = (held.clone(), present, dead);
        let (waiting, recent): (HashSet<String>, HashSet<String>) = self
            .db
            .call(move |conn| {
                let list = load_requested(conn)?;
                if list.is_empty() {
                    return Ok((HashSet::new(), HashSet::new()));
                }
                let p: HashSet<&str> = present_c.iter().map(String::as_str).collect();
                let h: HashSet<&str> = held_c.iter().map(String::as_str).collect();
                let d: HashSet<&str> = dead_c.iter().map(String::as_str).collect();
                let mut keep = Vec::new();
                let mut waiting = HashSet::new();
                let mut recent = HashSet::new();
                for r in list {
                    match judge_requested(&r, &p, &h, &d, rounds_known, now) {
                        RenewalOutcome::Waiting => {
                            waiting.extend(r.inputs.iter().cloned());
                            if now - r.at < LAST_RESORT_RETRY_SECS {
                                recent.extend(r.inputs.iter().cloned());
                            }
                            keep.push(r);
                        }
                        RenewalOutcome::Renewed => {
                            conn.execute(
                                "UPDATE settlements SET status = 'renewed', detail = ?2 WHERE id = ?1",
                                params![r.row, "renewed in an Ark round"],
                            )?;
                        }
                        RenewalOutcome::Failed(why) => {
                            conn.execute(
                                "UPDATE settlements SET status = 'failed', detail = ?2 WHERE id = ?1",
                                params![r.row, why],
                            )?;
                        }
                    }
                }
                save_requested(conn, &keep)?;
                Ok((waiting, recent))
            })
            .await?;
        held.extend(waiting);
        Ok((held, recent))
    }

    /// While an emergency withdrawal runs, renewals and sweeps would fight
    /// it over the same coins, so the whole cycle pauses. Also keeps the
    /// "ready to claim" flag (Home, phone alert) up to date, and notices when
    /// the withdrawal is over. `Some` = paused: `exiting` lists the coins
    /// being withdrawn when barkd could say (other coins are still renewed,
    /// so a payment that arrives meanwhile cannot expire); `None` there means
    /// barkd could not say, and nothing at all is touched.
    async fn exit_pause(&self) -> Result<Option<ExitPause>, AppError> {
        use crate::emergency_exit as ee;
        let started: Option<i64> = self
            .db
            .call(|c| crate::settings::get(c, ee::EXIT_STARTED_AT_KEY))
            .await?
            .and_then(|v| v.parse().ok());
        let live = match self.client.exits_live().await {
            Ok(v) => Some(ee::parse_live(&v)),
            Err(err) => {
                if started.is_some() {
                    tracing::warn!(error = %err, "settlement: exit status unavailable; staying paused");
                }
                None
            }
        };
        let now = now_unix();
        let claimable = live.as_deref().is_some_and(ee::any_claimable);
        // Over: started a while ago and barkd lists nothing still running.
        let over =
            started.is_some_and(|t| now - t > 120) && live.as_ref().is_some_and(Vec::is_empty);
        let known = live.is_some();
        let running = live.as_ref().is_some_and(|l| !l.is_empty());
        // barkd runs an exit nobody started from Settings (for example one
        // carried over in a restored wallet folder): adopt it, so Home shows
        // the red line while renewals and sweeps are paused.
        let adopt = running && started.is_none();
        self.db
            .call(move |c| {
                if known {
                    if claimable {
                        crate::settings::set(c, ee::EXIT_CLAIMABLE_KEY, "true")?;
                    } else {
                        delete_setting(c, ee::EXIT_CLAIMABLE_KEY)?;
                    }
                }
                if over {
                    delete_setting(c, ee::EXIT_STARTED_AT_KEY)?;
                    delete_setting(c, ee::EXIT_DESTINATION_KEY)?;
                }
                if adopt {
                    crate::settings::set(c, ee::EXIT_STARTED_AT_KEY, &now.to_string())?;
                }
                Ok(())
            })
            .await?;
        let paused = (started.is_some() && !over) || running;
        Ok(paused.then(|| ExitPause {
            reason: "An emergency withdrawal is in progress, so normal transfers are paused."
                .to_string(),
            exiting: live.map(|l| l.into_iter().map(|e| e.vtxo_id).collect()),
        }))
    }

    async fn cycle(&self, manual: bool) -> Result<TickReport, AppError> {
        let _one_at_a_time = CYCLE_LOCK.lock().await;
        let mut report = TickReport::default();
        let exit = self.exit_pause().await?;
        if let Some(ExitPause {
            reason,
            exiting: None,
        }) = &exit
        {
            // barkd cannot say which coins are being withdrawn: touch none.
            if manual {
                report.sweep_refused = Some(reason.clone());
            }
            return Ok(report);
        }
        let exiting: std::collections::HashSet<String> = exit
            .as_ref()
            .and_then(|e| e.exiting.clone())
            .unwrap_or_default();

        // NOTE: the sweep destination guard lives right before the sweep
        // section, NOT here. A missing destination disables SWEEPING, never
        // renewal — renewal is what keeps boarded/received funds alive, and
        // it must run regardless of where sweeps would go.

        let mut vtxos = self.client.vtxos().await.map_err(AppError::from)?;
        // Renewal only needs the coin list. A balance that cannot be read
        // right now must not stop it; the sweep further down needs it and
        // reports the error then.
        let mut balances = self.client.balances().await.map_err(AppError::from);
        if let Err(err) = &balances {
            tracing::warn!(error = %err, "settlement: balance unavailable; renewing anyway");
        }

        let now = now_unix();
        // How long before expiry a coin is refreshed. The Ark server's own
        // terms (coin lifetime, free refresh window) shape this; if they
        // cannot be read we keep the configured lead time unchanged.
        let terms = match self.client.ark_terms().await {
            Ok(t) => Some(t),
            Err(err) => {
                tracing::warn!(error = %err, "settlement: Ark server terms unavailable; using the configured renewal lead time");
                None
            }
        };
        // Extra renewals for coins the wallet flags (below) stay inside the
        // server's free window when one is known, so they never cost a fee.
        let topup_horizon_secs = terms
            .and_then(|t| t.free_refresh_below_blocks)
            .map(|b| (b - 1).max(0) * SECS_PER_BLOCK);
        let lead =
            refresh_trigger_blocks(self.config.renewal_lead_time_secs, terms) * SECS_PER_BLOCK;
        // The phone alert for coins near expiry is measured against this.
        // Best effort: nothing here may stop a renewal.
        let lead_text = lead.to_string();
        if let Err(err) = self
            .db
            .call(move |conn| {
                crate::settings::set(conn, crate::alerts::RENEWAL_TRIGGER_KEY, &lead_text)
            })
            .await
        {
            tracing::warn!(error = %err, "settlement: could not save the renewal point");
        }

        // 1. Renewal: refresh spendable VTXOs inside the renewal window,
        //    before their per-VTXO expiry. A VTXO barkd already refreshed
        //    drops out of the window by itself, so this stays quiet when
        //    renewal is already handled.
        //
        // BETA-HARDENING FIX (P0): barkd reports expiry as a block HEIGHT
        // (`expiry_height`), usually with no `expires_at`. The old filter
        // treated "no timestamp" as due, so (a) real VTXOs were refreshed on
        // EVERY tick regardless of expiry, and (b) never honoured the lead
        // time. We now convert heights with the chain tip. Genuinely unknown
        // expiry is still refreshed (fail safe) but at most once per
        // UNKNOWN_EXPIRY_RETRY_SECS, not every tick.
        let needs_tip = vtxos
            .spendable
            .iter()
            .any(|v| v.expires_at.is_none() && v.expiry_height.is_some());
        let tip = if needs_tip {
            match self.client.bitcoin_tip().await {
                Ok(tip) => Some(tip),
                Err(err) => {
                    tracing::warn!(error = %err, "settlement: chain tip unavailable; heights unresolved");
                    None
                }
            }
        } else {
            None
        };
        // Coins already waiting for a round are never asked for twice:
        // barkd 0.8.0 can still list them as spendable until the round
        // starts (up to an hour on mainnet), and Second's newer server
        // refuses a second request for them.
        let participations = match self.client.round_participations().await {
            Ok(p) => Some(p),
            Err(err) => {
                tracing::debug!(error = %err, "settlement: round list unavailable");
                None
            }
        };
        let (mut excluded, recently_requested) = self
            .settle_requested_renewals(&vtxos, participations.as_deref(), now)
            .await?;
        // Coins already in a round (or a renewal still waiting): the sweep below
        // must not try to send them, because the wallet would refuse.
        let mut in_round = excluded.clone();
        // Last resort: a coin close to expiry is asked for again (at most
        // hourly) even when a round is supposed to hold it. Coins in an
        // emergency withdrawal are never touched.
        let critical = lead / 4;
        excluded.retain(|id| {
            let near_expiry = vtxos
                .spendable
                .iter()
                .find(|v| &v.id == id)
                .and_then(|v| vtxo_expiry_unix(v, tip.as_ref()))
                .is_some_and(|t| t - now <= critical);
            !(near_expiry && !recently_requested.contains(id))
        });
        excluded.extend(exiting.iter().cloned());

        let mut unknown_expiry = false;
        let mut due: Vec<_> = Vec::new();
        for v in vtxos.spendable.iter().filter(|v| !excluded.contains(&v.id)) {
            match vtxo_expiry_unix(v, tip.as_ref()) {
                Some(t) if t - now <= lead => due.push(v),
                Some(_) => {}
                None => {
                    unknown_expiry = true;
                    due.push(v);
                }
            }
        }
        if unknown_expiry {
            // Throttle: only retry unknown-expiry refreshes hourly.
            let mut last = self
                .last_unknown_refresh
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let throttled = matches!(*last, Some(t) if now - t < UNKNOWN_EXPIRY_RETRY_SECS);
            if throttled {
                // Keep only VTXOs whose expiry IS known and due.
                due.retain(|v| vtxo_expiry_unix(v, tip.as_ref()).is_some());
            } else {
                *last = Some(now);
            }
        }
        // What made these coins due (kept for the note on the upkeep line).
        let timed_n = due.len();
        let timed_soonest_secs = due
            .iter()
            .filter_map(|v| vtxo_expiry_unix(v, tip.as_ref()))
            .min()
            .map(|t| t - now);
        // The wallet's own flag wins: coins it says "have to be refreshed
        // before they can be sent again" (needs_refresh_sat) are refreshed
        // now, soonest-to-expire first, even if our timing has not reached
        // them yet. Otherwise they could not be swept.
        let flagged = balances.as_ref().map(|b| b.needs_refresh_sats).unwrap_or(0);
        let mut covered: i64 = due.iter().map(|v| v.amount_sats).sum();
        if flagged > covered {
            let mut rest: Vec<(&crate::wallet::barkd::Vtxo, i64)> = vtxos
                .spendable
                .iter()
                .filter(|v| !due.iter().any(|d| d.id == v.id))
                .filter(|v| !excluded.contains(&v.id))
                .filter_map(|v| vtxo_expiry_unix(v, tip.as_ref()).map(|t| (v, t)))
                .filter(|(_, t)| topup_horizon_secs.is_none_or(|h| t - now <= h))
                .collect();
            rest.sort_by_key(|(_, t)| *t);
            for (v, _) in rest {
                if covered >= flagged {
                    break;
                }
                covered += v.amount_sats;
                due.push(v);
            }
        }
        let flagged_n = due.len().saturating_sub(timed_n);
        // Less than the wallet can renew: add the server's other coins (they
        // are merged into one fresh coin) if that reaches the minimum;
        // otherwise skip quietly. Nothing is recorded as failed and no alert
        // is raised for an amount that cannot be renewed at all; Home says
        // so in one plain line.
        let mut too_small: Option<(i64, Option<i64>)> = None;
        let due_total: i64 = due.iter().map(|v| v.amount_sats).sum();
        if due_total > 0 && due_total < MIN_RENEW_SATS {
            let mut extra: Vec<&crate::wallet::barkd::Vtxo> = vtxos
                .spendable
                .iter()
                .filter(|v| !v.id.is_empty())
                .filter(|v| !due.iter().any(|d| d.id == v.id))
                .filter(|v| !excluded.contains(&v.id))
                .collect();
            // Coins inside the server's free renewal window first (they cost
            // nothing), then the smallest. A coin that would pay a renewal
            // fee is only added while that fee stays below what is rescued.
            let free = |v: &crate::wallet::barkd::Vtxo| {
                topup_horizon_secs.is_some_and(|h| {
                    vtxo_expiry_unix(v, tip.as_ref()).is_some_and(|t| t - now <= h)
                })
            };
            extra.sort_by_key(|v| (!free(v), v.amount_sats));
            let mut total = due_total;
            for v in extra {
                if total >= MIN_RENEW_SATS {
                    break;
                }
                if !free(v) && v.amount_sats > MERGE_MAX_PAID_MULTIPLE * due_total {
                    continue;
                }
                total += v.amount_sats;
                due.push(v);
            }
            if total < MIN_RENEW_SATS {
                let lapse = due
                    .iter()
                    .filter_map(|v| vtxo_expiry_unix(v, tip.as_ref()))
                    .min();
                too_small = Some((total, lapse));
                due.clear();
            }
        }
        let merged_n = due.len().saturating_sub(timed_n + flagged_n);
        let renew_why = renew_reason(timed_n, timed_soonest_secs, flagged_n, merged_n);
        // Best effort: a note for Home must never stop a renewal.
        if let Err(err) = self
            .db
            .call(move |conn| {
                match too_small {
                    Some((sats, lapse)) => {
                        crate::settings::set(conn, TOO_SMALL_SATS_KEY, &sats.to_string())?;
                        match lapse {
                            Some(at) => {
                                crate::settings::set(conn, TOO_SMALL_LAPSE_AT_KEY, &at.to_string())?
                            }
                            None => delete_setting(conn, TOO_SMALL_LAPSE_AT_KEY)?,
                        }
                    }
                    None => {
                        delete_setting(conn, TOO_SMALL_SATS_KEY)?;
                        delete_setting(conn, TOO_SMALL_LAPSE_AT_KEY)?;
                    }
                }
                Ok(())
            })
            .await
        {
            tracing::warn!(error = %err, "settlement: could not save the too-small note");
        }

        // At most MAX_REFRESH_COINS per request, soonest-expiring first; the
        // rest are still due and go in on the next cycle.
        if due.len() > MAX_REFRESH_COINS {
            due.sort_by_key(|v| vtxo_expiry_unix(v, tip.as_ref()).unwrap_or(i64::MAX));
            due.truncate(MAX_REFRESH_COINS);
        }
        let due_ids: Vec<String> = due
            .iter()
            .map(|v| v.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let mut renewal_accepted = false;
        if !due_ids.is_empty() {
            match self.client.refresh_vtxos(&due_ids).await {
                Ok(_) => {
                    in_round.extend(due_ids.iter().cloned());
                    // Accepted means registered for the next round, not done
                    // yet: "requested" until the old coins are replaced
                    // (checked on the following cycles).
                    let amount: i64 = due.iter().map(|v| v.amount_sats).sum();
                    let count = due_ids.len();
                    let inputs = due_ids.clone();
                    self.db
                        .call(move |conn| {
                            let row = record_id(
                                conn,
                                "renew",
                                amount,
                                "", // renewal does not send anywhere; no destination to record
                                "requested",
                                Some(&format!(
                                    "asked to renew {count} coins in the Ark server's next round. Why: {renew_why}"
                                )),
                            )?;
                            let mut list = load_requested(conn)?;
                            list.push(RequestedRenewal {
                                row,
                                inputs,
                                at: now_unix(),
                            });
                            save_requested(conn, &list)
                        })
                        .await?;
                    report.refreshed_vtxos = count;
                    renewal_accepted = true;
                }
                Err(err) if is_already_in_round(&err.to_string()) => {
                    // Already being renewed: not a failure, no alert. One
                    // row while it lasts, not one per cycle.
                    let reason = err.to_string();
                    tracing::info!(error = %reason, "vtxo refresh: coins already in a round");
                    let amount: i64 = due.iter().map(|v| v.amount_sats).sum();
                    self.db
                        .call(move |conn| {
                            let newest: Option<String> = conn
                                .query_row(
                                    "SELECT status FROM settlements WHERE kind = 'renew'
                                     ORDER BY created_at DESC, rowid DESC LIMIT 1",
                                    [],
                                    |r| r.get(0),
                                )
                                .ok();
                            if newest.as_deref() != Some("waiting") {
                                record(
                                    conn,
                                    "renew",
                                    amount,
                                    "",
                                    "waiting",
                                    Some("already being renewed in an Ark round"),
                                )?;
                            }
                            Ok(())
                        })
                        .await?;
                }
                Err(err) => {
                    // Recorded (not only logged): Home warns after a few
                    // failures in a row, and that count is read from here.
                    let reason = err.to_string();
                    tracing::warn!(error = %reason, "vtxo refresh failed");
                    let amount: i64 = due.iter().map(|v| v.amount_sats).sum();
                    self.db
                        .call(move |conn| {
                            record(conn, "renew", amount, "", "failed", Some(&reason))
                        })
                        .await?;
                }
            }
        }
        if let Some(e) = exit {
            // An emergency withdrawal is running: coins outside it were
            // renewed above; nothing is sent anywhere until it is over.
            if manual {
                report.sweep_refused = Some(e.reason);
            }
            return Ok(report);
        }
        if renewal_accepted {
            // The wallet accepted a renewal: the balance read above may still
            // count the coins it locked. The timed round sweeps on the next
            // cycle instead of sending from a stale balance (which would
            // fail and alarm). The owner's button reads everything again and
            // goes ahead with what is spendable now. A renewal that FAILED
            // never holds a sweep back: that is exactly when the owner needs
            // "move my money now" to work.
            if !manual {
                return Ok(report);
            }
            vtxos = self.client.vtxos().await.map_err(AppError::from)?;
            balances = self.client.balances().await.map_err(AppError::from);
        }
        let balances = balances?;

        // 2. Sweep: move the free balance to the
        //    merchant's own wallet. Requires a configured destination; an
        //    unset/blank one means sweeping stays off (renewal above is
        //    unaffected).
        let cfg_destination = self.config.settlement_destination.clone();
        let Some(destination) = self
            .db
            .call(move |conn| crate::payout::effective(conn, cfg_destination.as_deref()))
            .await?
            .filter(|d| !d.trim().is_empty())
        else {
            return Ok(report);
        };

        // Coins locked in a renewal round are not sendable yet. The wallet may
        // still count them as spendable for a while (up to about an hour), and
        // a send that picks one fails. An automatic sweep waits until the round
        // is done (at most a couple of hours); the owner's button sends what is
        // free now.
        let locked_sats: i64 = vtxos
            .spendable
            .iter()
            .filter(|v| in_round.contains(&v.id))
            .map(|v| v.amount_sats)
            .sum();
        report.held_back = held_back_parts(locked_sats, &balances);
        if locked_sats > 0 && !manual {
            self.forget_refusal();
            return Ok(report);
        }
        let free_coins: Vec<crate::wallet::barkd::Vtxo> = vtxos
            .spendable
            .iter()
            .filter(|v| !in_round.contains(&v.id))
            .cloned()
            .collect();
        // A transfer whose outcome is unknown (interrupted or timed out). This is
        // checked BEFORE anything about the amount, so it is settled or alerted
        // even when the till is now empty (the send may have taken everything).
        // Try to
        // settle it from the wallet's history; if that is not clear, send nothing,
        // automatic or button, until the owner says how it ended.
        if let Some(unsettled) = self.db.call(|c| unsettled_send(c)).await? {
            let free_ids: Vec<String> = free_coins.iter().map(|v| v.id.clone()).collect();
            if !self.settle_from_history(&unsettled, &free_ids).await? {
                self.db
                    .call(|c| {
                        // Make sure a restart-left "sending" line is shown as unknown.
                        c.execute(
                            "UPDATE settlements SET status = 'unknown' WHERE status = 'sending'",
                            [],
                        )?;
                        Ok(())
                    })
                    .await?;
                self.alert_transfer_unknown().await?;
                report.sweep_refused = Some(TRANSFER_UNKNOWN_NOTE.to_string());
                return Ok(report);
            }
        }
        let offchain = balances.offchain_sats.saturating_sub(locked_sats);
        // Nothing stays behind: everything that is free is sent.
        let excess = offchain;
        // Nothing to send (or not yet worth sending) ends any refusal being
        // remembered, so the same refusal later is recorded again instead of
        // being taken for the old one.
        if excess <= 0 {
            self.forget_refusal();
            if manual && locked_sats > 0 {
                report.sweep_refused = Some(
                    "The Ark server is renewing your coins right now, so they cannot be sent for a little while (usually under an hour). Nothing is lost: try again later, or the automatic transfer sends them once the renewal is done.".to_string(),
                );
            }
            return Ok(report);
        }
        // The Ark server refused this wallet's software as too old a little while
        // ago: do not keep asking every few minutes (the button still tries).
        if !manual
            && self
                .db
                .call(|c| Ok(update_needed_paused(c, now_unix())))
                .await?
        {
            tracing::info!("automatic transfer paused: the wallet software needs an update");
            return Ok(report);
        }
        // An automatic sweep waits until the excess is worth sending, so the
        // payout wallet receives a few bigger coins instead of many tiny ones.
        // "Move my money now" always sends whatever is there.
        // The owner's number from Settings for this kind of address (never
        // under its floor), else the floor. When it is reached, the whole free
        // balance goes in one send.
        let rail = crate::payout::Rail::of(&destination);
        let min_excess = self
            .db
            .call(move |c| crate::payout::sweep_min(c, rail))
            .await?;
        if !manual && excess < min_excess {
            self.forget_refusal();
            return Ok(report);
        }

        // Last line of defence: re-check the address at the moment of use (the
        // same checks as when it was saved: checksum, form, network). Anything
        // that fails is refused and shown on the dashboard, never sent.
        let mut refused = crate::payout::check(&destination, &self.config.mode)
            .err()
            .map(|why| format!("payout address refused: {why}"));
        // An Ark address for a different Ark server is never sent to: only a
        // definite mismatch refuses; if the server key or the address cannot
        // be read, barkd's own checks decide.
        if refused.is_none() && crate::config::is_ark_destination(&destination) {
            if let Ok(Some(key)) = self.client.ark_server_pubkey().await {
                if crate::payout::ark_address_on_server(&destination, &key) == Some(false) {
                    refused = Some(
                        format!(
                            "payout address refused: it is an Ark address on a different Ark server than this wallet uses, so nothing was sent. {} Then save the new address under Settings.",
                            crate::payout::ark_server_advice(&self.config.mode)
                        ),
                    );
                }
            }
        }
        if let Some(reason) = refused {
            tracing::error!(%reason, "sweep refused");
            report.sweep_refused = Some(reason.clone());
            let already = {
                let mut last = self.last_refusal.lock().unwrap_or_else(|e| e.into_inner());
                let same = last.as_deref() == Some(reason.as_str());
                if !same {
                    *last = Some(reason.clone());
                }
                same
            };
            if !already {
                let dest = destination.clone();
                self.db
                    .call(move |conn| {
                        record(
                            conn,
                            "sweep_refused",
                            excess,
                            &dest,
                            "failed",
                            Some(&reason),
                        )
                    })
                    .await?;
            }
            return Ok(report);
        }

        // Route Ark addresses to the Ark-send path; anything else is treated
        // as an on-chain destination.
        let ark = crate::config::is_ark_destination(&destination);
        let kind = if ark { "sweep_ark" } else { "sweep_onchain" };

        // 3. On-chain fee limit. An automatic on-chain withdrawal waits for
        //    cheap fees; Ark sweeps (almost free) and manual sweeps do not.
        //    If the fee rate cannot be read we postpone: never overpay blind.
        if !ark && !manual {
            let max = self.config.onchain_sweep_max_fee_sat_vb;
            let waiting = match self.client.onchain_fee_rates().await {
                Ok(r) if r.regular_sat_per_vb <= max => None,
                Ok(_) => Some(format!(
                    "waiting for on-chain fees to drop to {max} sat/vB or less"
                )),
                Err(err) => {
                    tracing::warn!(error = %err, "could not read on-chain fee rates");
                    Some("could not read on-chain fee rates; withdrawal postponed".to_string())
                }
            };
            if let Some(reason) = waiting {
                tracing::info!(%reason, "on-chain sweep postponed");
                report.sweep_refused = Some(reason.clone());
                let already = {
                    let mut last = self.last_refusal.lock().unwrap_or_else(|e| e.into_inner());
                    let same = last.as_deref() == Some(reason.as_str());
                    if !same {
                        *last = Some(reason.clone());
                    }
                    same
                };
                if !already {
                    let dest = destination.clone();
                    self.db
                        .call(move |conn| {
                            record(conn, kind, excess, &dest, "waiting", Some(&reason))
                        })
                        .await?;
                }
                return Ok(report);
            }
        }
        // (A refusal remembered from an earlier check is forgotten only once a
        // send really goes ahead, below, so a held-back send is written once.)
        let (amount, send_id, result) = if ark {
            if excess < ARK_MIN_SEND_SATS {
                let reason = format!(
                    "Only {excess} sats are free, which is too small to send (the smallest payment is {ARK_MIN_SEND_SATS} sats). Nothing was sent: it stays in your till and goes out with the next payments."
                );
                return self
                    .hold_send(report, kind, excess, &destination, reason)
                    .await;
            }
            self.forget_refusal();
            let send_id = self.begin_send(kind, excess, &destination, &[]).await?;
            (
                excess,
                send_id,
                self.client.ark_send(&destination, excess).await,
            )
        } else {
            // R8: offboard only enough whole VTXOs to cover the excess
            // (largest first) via /wallet/offboard/vtxos, leaving the rest
            // as the till. Overshoot is at most one VTXO.
            let (ids, total) = select_vtxos_covering(&free_coins, excess);
            if ids.is_empty() {
                return Ok(report);
            }
            // Net-amount check: never send an amount the network fee would eat
            // (or leave as a dust coin). Held back, nothing leaves the till.
            let fee_rate = match self.client.onchain_fee_rates().await {
                Ok(r) => r.regular_sat_per_vb,
                Err(_) => UNREADABLE_FEE_GUESS_SAT_VB,
            };
            if let Some(reason) = onchain_arrival_problem(total, fee_rate) {
                return self
                    .hold_send(report, kind, total, &destination, reason)
                    .await;
            }
            self.forget_refusal();
            let send_id = self.begin_send(kind, total, &destination, &ids).await?;
            (
                total,
                send_id,
                self.client.offboard_vtxos(&destination, &ids).await,
            )
        };

        match result {
            Ok(receipt) => {
                let detail = serde_json::to_string(&receipt).unwrap_or_else(|_| "{}".to_string());
                self.db
                    .call(move |conn| {
                        conn.execute(
                            "UPDATE settlements SET status = 'broadcast', detail = ?1 WHERE id = ?2",
                            params![detail, send_id],
                        )?;
                        // A send went through: the software is not too old.
                        crate::settings::set(conn, WALLET_UPDATE_NEEDED_KEY, "0")
                    })
                    .await?;
                report.swept_sats = Some(amount);
                tracing::info!(kind, amount_sats = amount, "settlement sweep sent");
            }
            Err(err) => {
                let mut reason = err.to_string();
                tracing::warn!(error = %reason, "settlement sweep failed");
                let too_old = is_update_needed_error(&reason);
                if !too_old && send_outcome_unknown(&err) {
                    // The request may have gone through. Do not call it failed
                    // and do not try again: write it down as unknown and stop
                    // sending until it is settled (from the wallet's history,
                    // or by the owner).
                    let note = reason.clone();
                    self.db
                        .call(move |conn| {
                            conn.execute(
                                "UPDATE settlements SET status = 'unknown', detail = COALESCE(?1 || ' ' || detail, ?1) WHERE id = ?2",
                                params![note, send_id],
                            )?;
                            Ok(())
                        })
                        .await?;
                    report.sweep_refused = Some(TRANSFER_UNKNOWN_NOTE.to_string());
                    self.alert_transfer_unknown().await?;
                    return Ok(report);
                }
                // A clear "no": nothing left the till, so the journal line goes.
                self.db
                    .call(move |conn| {
                        conn.execute("DELETE FROM settlements WHERE id = ?1", params![send_id])?;
                        Ok(())
                    })
                    .await?;
                if too_old {
                    reason = format!(
                        "Your wallet software is too old for the Ark server and needs an update. Your money is safe. ({reason})"
                    );
                }
                let dest = destination.clone();
                self.db
                    .call(move |conn| {
                        record_failure(conn, kind, amount, &dest, &reason)?;
                        if too_old {
                            mark_update_needed(conn, now_unix())?;
                        }
                        Ok(())
                    })
                    .await?;
            }
        }

        Ok(report)
    }
}

impl SettlementEngine {
    /// Write the "sending" line BEFORE the irreversible call, so a crash or a
    /// timeout leaves a trace instead of nothing.
    async fn begin_send(
        &self,
        kind: &'static str,
        amount: i64,
        destination: &str,
        vtxo_ids: &[String],
    ) -> Result<String, AppError> {
        let detail = serde_json::json!({ "vtxos": vtxo_ids }).to_string();
        let dest = destination.to_string();
        self.db
            .call(move |conn| record_id(conn, kind, amount, &dest, "sending", Some(&detail)))
            .await
    }

    /// One phone alert per unknown transfer, not one per check.
    async fn alert_transfer_unknown(&self) -> Result<(), AppError> {
        self.db
            .call(|conn| {
                let Some(u) = unsettled_send(conn)? else {
                    return Ok(());
                };
                if crate::settings::get(conn, TRANSFER_UNKNOWN_ALERTED_KEY)?.as_deref()
                    != Some(u.id.as_str())
                {
                    crate::alerts::queue_notice(conn, "transfer_unknown")?;
                    crate::settings::set(conn, TRANSFER_UNKNOWN_ALERTED_KEY, &u.id)?;
                }
                Ok(())
            })
            .await
    }

    /// Try to settle an unknown transfer without bothering the owner: if the
    /// wallet's history shows exactly one matching outbound movement (same
    /// destination, about the amount, recent enough) it went out; if it was an
    /// on-chain send and every coin it named is still free in the wallet well
    /// after the attempt, it did not. Anything less clear stays unknown.
    async fn settle_from_history(
        &self,
        u: &UnsettledSend,
        free_coin_ids: &[String],
    ) -> Result<bool, AppError> {
        let Ok(page) = self.client.history().await else {
            return Ok(false);
        };
        let matches = count_history_matches(u, &page.entries, now_unix());
        let id = u.id.clone();
        if matches == 1 {
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE settlements SET status = 'broadcast', detail = 'Confirmed from the wallet history after an interruption.' WHERE id = ?1",
                        params![id],
                    )?;
                    Ok(())
                })
                .await?;
            return Ok(true);
        }
        // "Not sent" is concluded only on positive evidence from the wallet,
        // never from time alone: every coin is still free, the wallet lists
        // none of them in a round, and its history shows no movement at all
        // that could be this send.
        if matches == 0
            && u.kind == "sweep_onchain"
            && !u.vtxos.is_empty()
            && now_unix() - u.created_at >= OFFBOARD_NOT_SENT_AFTER_SECS
            && u.vtxos.iter().all(|v| free_coin_ids.contains(v))
            && !any_history_trace(u, &page.entries)
            && rounds_clear_of(
                &u.vtxos,
                self.client.round_participations().await.ok().as_deref(),
            )
        {
            self.db
                .call(move |conn| {
                    conn.execute(
                        "UPDATE settlements SET status = 'failed', detail = 'The coins are still in the wallet, so nothing was sent.' WHERE id = ?1",
                        params![id],
                    )?;
                    Ok(())
                })
                .await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// A send that is held back on purpose (too small to be worth it): shown as
    /// the sweep's plain-words note, and written once as a "waiting" line, not
    /// once per check. Nothing leaves the till.
    async fn hold_send(
        &self,
        mut report: TickReport,
        kind: &'static str,
        amount: i64,
        destination: &str,
        reason: String,
    ) -> Result<TickReport, AppError> {
        tracing::info!(%reason, kind, "sweep held back");
        report.sweep_refused = Some(reason.clone());
        let already = {
            let mut last = self.last_refusal.lock().unwrap_or_else(|e| e.into_inner());
            let same = last.as_deref() == Some(reason.as_str());
            if !same {
                *last = Some(reason.clone());
            }
            same
        };
        if !already {
            let dest = destination.to_string();
            self.db
                .call(move |conn| record(conn, kind, amount, &dest, "waiting", Some(&reason)))
                .await?;
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
    record_id(conn, kind, amount_sats, destination, status, detail).map(|_| ())
}

/// Record a failed sweep send.
///
/// The same failure repeating every cycle (same kind, same destination, same
/// reason, and nothing recorded for that kind since) does not add a line each
/// time. It refreshes the existing line instead: new time and current amount.
/// So the activity list does not fill up, while Home and the phone alerts, which
/// read the newest line, keep seeing the failure for as long as it lasts. A
/// different reason, or any other outcome in between (a sent sweep, a wait for
/// fees), starts a new line as before.
fn record_failure(
    conn: &Connection,
    kind: &str,
    amount_sats: i64,
    destination: &str,
    reason: &str,
) -> Result<(), AppError> {
    let latest: Option<(String, String, Option<String>, String)> = conn
        .query_row(
            "SELECT id, status, detail, destination FROM settlements
             WHERE kind = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![kind],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some((id, status, detail, dest)) = latest {
        if status == "failed" && detail.as_deref() == Some(reason) && dest == destination {
            conn.execute(
                "UPDATE settlements SET created_at = ?1, amount_sats = ?2 WHERE id = ?3",
                params![now_unix(), amount_sats, id],
            )?;
            return Ok(());
        }
    }
    record(conn, kind, amount_sats, destination, "failed", Some(reason))
}

/// [`record`], returning the new row's id.
fn record_id(
    conn: &Connection,
    kind: &str,
    amount_sats: i64,
    destination: &str,
    status: &str,
    detail: Option<&str>,
) -> Result<String, AppError> {
    let id = new_id("stl");
    let now = now_unix();
    // The dollar price at this moment, for the owner's bookkeeping. Blank when
    // no recent price is saved: never guessed.
    let (fiat_currency, fiat_amount, rate, rate_source) =
        match crate::rates::saved_quote(conn, "USD", now) {
            Some(q) => (
                Some(q.currency),
                Some(format!(
                    "{:.2}",
                    amount_sats as f64 / 100_000_000.0 * q.fiat_per_btc
                )),
                Some(format!("{:.2}", q.fiat_per_btc)),
                Some(q.source),
            ),
            None => (None, None, None, None),
        };
    conn.execute(
        "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at,
                                  fiat_currency, fiat_amount, rate, rate_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            kind,
            amount_sats,
            destination,
            status,
            detail,
            now,
            fiat_currency,
            fiat_amount,
            rate,
            rate_source
        ],
    )?;
    Ok(id)
}

fn delete_setting(conn: &Connection, key: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}

fn load_requested(conn: &Connection) -> Result<Vec<RequestedRenewal>, AppError> {
    let Some(text) = crate::settings::get(conn, REQUESTED_RENEWALS_KEY)? else {
        return Ok(Vec::new());
    };
    // Unreadable: start over with an empty list (the wallet's own round list
    // and locked balance still keep coins in a round from being swept), but
    // say so in the log instead of silently.
    Ok(serde_json::from_str(&text).unwrap_or_else(|err| {
        tracing::error!(error = %err, "saved renewal requests could not be read; starting a fresh list");
        Vec::new()
    }))
}

fn save_requested(conn: &Connection, list: &[RequestedRenewal]) -> Result<(), AppError> {
    if list.is_empty() {
        return delete_setting(conn, REQUESTED_RENEWALS_KEY);
    }
    let text = serde_json::to_string(list)
        .map_err(|e| AppError::Internal(format!("could not save requested renewals: {e}")))?;
    crate::settings::set(conn, REQUESTED_RENEWALS_KEY, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::barkd::mock;

    fn test_config(destination: Option<&str>) -> Arc<Config> {
        Arc::new(Config {
            settlement_destination: destination.map(str::to_string),
            renewal_lead_time_secs: 604_800,
            ..Config::default()
        })
    }

    /// An engine that sends to `destination` (none: sweeping stays off).
    fn test_engine(
        client: crate::wallet::barkd::BarkdClient,
        db: Db,
        destination: Option<&str>,
    ) -> SettlementEngine {
        SettlementEngine::new(client, db, test_config(destination))
    }

    #[tokio::test]
    async fn sweeps_to_the_payout_address_saved_in_settings() {
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            "/api/v1/wallet/send" => (200, r#"{"sent": true}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        // No address in the server configuration; one saved through the page.
        db.call(|c| {
            crate::payout::set_initial(
                c,
                "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh",
                "test",
            )
        })
        .await
        .unwrap();
        let engine = test_engine(client, db.clone(), None);
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));
        let destination = {
            let requests = recorded.lock().unwrap();
            let send = requests
                .iter()
                .find(|r| r.route() == "/api/v1/wallet/send")
                .expect("send recorded");
            let body: serde_json::Value = serde_json::from_str(&send.body).unwrap();
            body["destination"].clone()
        };
        assert_eq!(
            destination,
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"
        );
    }

    #[tokio::test]
    async fn sweeps_the_whole_till_via_ark_send() {
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
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
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));

        // Scope the guard so it is dropped before the awaited DB call below
        // (clippy::await_holding_lock; also just correct hygiene).
        let (send_is_post, destination, amount_sats) = {
            let requests = recorded.lock().unwrap();
            let send = requests
                .iter()
                .find(|r| r.route() == "/api/v1/wallet/send")
                .expect("send recorded");
            let body: serde_json::Value = serde_json::from_str(&send.body).unwrap();
            (
                send.method == "POST",
                body["destination"].clone(),
                body["amount_sat"].clone(),
            )
        };
        assert!(send_is_post);
        assert_eq!(
            destination,
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"
        );
        assert_eq!(amount_sats, 150_000);

        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_ark' AND status = 'broadcast' AND amount_sats = 150000",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn an_address_for_the_wrong_network_is_refused_and_nothing_is_sent() {
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
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
            "/api/v1/wallet/offboard/vtxos" => (200, r#"{"ok": true}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        // A real-money (mainnet) address on a test-mode server.
        let engine = test_engine(
            client,
            db.clone(),
            Some("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, None);
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap_or("")
            .contains("test mode"));
        {
            let requests = recorded.lock().unwrap();
            assert!(
                !requests
                    .iter()
                    .any(|r| r.route().contains("send") || r.route().contains("offboard")),
                "nothing may be sent to a refused address"
            );
        }
        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_refused' AND status = 'failed'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
        // A second tick does not pile up identical rows.
        engine.tick().await.unwrap();
        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_refused'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    /// A refusal that stopped mattering (nothing left to send) and then comes
    /// back is recorded again: Home and the alerts must not miss the second one.
    #[tokio::test]
    async fn a_refusal_that_comes_back_after_a_quiet_spell_is_recorded_again() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let high = Arc::new(AtomicBool::new(true));
        let high_in_mock = high.clone();
        let (base, _recorded) = mock::spawn(move |req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => {
                let sats = if high_in_mock.load(Ordering::SeqCst) { 150_000 } else { 1_000 };
                (200, format!(r#"{{"spendable_sat": {sats}}}"#))
            }
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        // A real-money address on a test-mode server: always refused.
        let engine = test_engine(
            client,
            db.clone(),
            Some("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
        );
        let count = |db: Db| async move {
            db.call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_refused'",
                    [],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap()
        };
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert_eq!(count(db.clone()).await, 1, "the same refusal is one line");
        // The balance falls under the minimum: nothing to send for a while.
        high.store(false, Ordering::SeqCst);
        engine.tick().await.unwrap();
        // Money arrives again and the same address is refused again.
        high.store(true, Ordering::SeqCst);
        engine.tick().await.unwrap();
        assert_eq!(
            count(db.clone()).await,
            2,
            "the returning refusal is recorded"
        );
    }

    /// A send that keeps failing is one line that stays fresh, not a new line
    /// every cycle; a new reason or a success in between starts a new line.
    #[tokio::test]
    async fn a_send_that_keeps_failing_is_one_line_that_stays_fresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // 0 = error A, 1 = error B, 2 = the send works.
        let mode = Arc::new(AtomicUsize::new(0));
        let mode_in_mock = mode.clone();
        let (base, _recorded) = mock::spawn(move |req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            "/api/v1/wallet/send" => match mode_in_mock.load(Ordering::SeqCst) {
                0 => (500, "problem A".to_string()),
                1 => (500, "problem B".to_string()),
                _ => (200, r#"{"sent": true}"#.to_string()),
            },
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let count = |status: &'static str| {
            let db = db.clone();
            async move {
                db.call(move |conn| {
                    Ok(conn.query_row(
                        "SELECT COUNT(*) FROM settlements WHERE kind = 'sweep_ark' AND status = ?1",
                        params![status],
                        |r| r.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap()
            }
        };

        // The same failure three times in a row: one line.
        for _ in 0..3 {
            let report = engine.tick().await.unwrap();
            assert_eq!(report.swept_sats, None);
        }
        assert_eq!(count("failed").await, 1);

        // Make that line old, then fail again: still one line, and it is
        // fresh again (so Home still shows the failure as the latest event).
        db.call(|conn| {
            conn.execute("UPDATE settlements SET created_at = created_at - 1000", [])?;
            Ok(())
        })
        .await
        .unwrap();
        engine.tick().await.unwrap();
        assert_eq!(count("failed").await, 1);
        let newest: i64 = db
            .call(|conn| {
                Ok(conn.query_row("SELECT MAX(created_at) FROM settlements", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert!(
            newest >= now_unix() - 100,
            "the repeated failure was not refreshed"
        );

        // A different reason is a new line.
        mode.store(1, Ordering::SeqCst);
        engine.tick().await.unwrap();
        assert_eq!(count("failed").await, 2);

        // The send works: a sent line. Failing again with the first reason is
        // a new line, because something else happened in between.
        mode.store(2, Ordering::SeqCst);
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));
        assert_eq!(count("broadcast").await, 1);
        mode.store(0, Ordering::SeqCst);
        engine.tick().await.unwrap();
        assert_eq!(count("failed").await, 3);
    }

    #[tokio::test]
    async fn onchain_destinations_offboard_only_the_excess_vtxos() {
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (
                200,
                r#"{"spendable_sat": 150000}"#.to_string(),
            ),
            "/api/v1/fees/onchain" => (200, r#"{"fast_sat_per_vb": 20, "regular_sat_per_vb": 10, "slow_sat_per_vb": 5}"#.to_string()),
            "/api/v1/wallet/offboard/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));

        let requests = recorded.lock().unwrap();
        let offboard = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/offboard/vtxos")
            .expect("offboard recorded");
        assert_eq!(offboard.method, "POST");
        let body: serde_json::Value = serde_json::from_str(&offboard.body).unwrap();
        assert_eq!(
            body["address"],
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        );
        assert_eq!(body["vtxos"][0], "v1");
        assert!(
            !requests
                .iter()
                .any(|r| r.route() == "/api/v1/wallet/offboard/all"),
            "must not offboard the whole till"
        );
    }

    #[tokio::test]
    async fn money_on_the_server_for_emergency_fees_is_never_moved_by_a_sweep() {
        // 20,000 sats of regular Bitcoin sit on the server (the money an
        // emergency withdrawal would pay its fees from). A sweep, manual or
        // automatic, may only offboard Ark coins: nothing may touch that balance.
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#.to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            "/api/v1/fees/onchain" => (200, r#"{"fast_sat_per_vb": 20, "regular_sat_per_vb": 10, "slow_sat_per_vb": 5}"#.to_string()),
            "/api/v1/wallet/offboard/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"),
        );
        engine.tick().await.unwrap();
        engine.sweep_now().await.unwrap();

        let requests = recorded.lock().unwrap();
        let moving: Vec<String> = requests
            .iter()
            .filter(|r| r.method != "GET")
            .map(|r| r.route().to_string())
            .filter(|r| r != "/api/v1/onchain/sync")
            .collect();
        assert!(
            moving.iter().all(|r| r == "/api/v1/wallet/offboard/vtxos"),
            "a sweep may only offboard Ark coins, saw {moving:?}"
        );
        for banned in [
            "/onchain/drain",
            "/onchain/send",
            "/wallet/offboard/all",
            "/exits/",
        ] {
            assert!(
                !requests
                    .iter()
                    .any(|r| r.method != "GET" && r.route().contains(banned)),
                "a sweep must never POST to {banned}"
            );
        }
    }

    // ---- the journal for sends whose outcome is unknown ----

    const JOURNAL_ARK: &str =
        "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";

    /// A wallet holding 50,000 sats in one Ark coin. `send_mode`: 0 = the send
    /// answer cannot be read (outcome unknown), 1 = refused (a clear no), 2 = ok.
    /// `in_history` puts a matching send in the wallet's history.
    fn journal_mock(
        send_mode: Arc<std::sync::atomic::AtomicUsize>,
        in_history: Arc<std::sync::atomic::AtomicBool>,
    ) -> (
        String,
        Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        use std::sync::atomic::Ordering;
        mock::spawn(move |req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 50000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 50000}"#.to_string()),
            "/api/v1/wallet/rounds" => (200, "[]".to_string()),
            "/api/v1/wallet/send" => match send_mode.load(Ordering::SeqCst) {
                0 => (200, "this is not json".to_string()),
                1 => (400, r#"{"message": "bad destination"}"#.to_string()),
                _ => (200, r#"{"message": "ok"}"#.to_string()),
            },
            r if r.contains("history") => {
                if in_history.load(Ordering::SeqCst) {
                    let now = chrono::Utc::now().to_rfc3339();
                    (
                        200,
                        format!(
                            r#"{{"history": [{{"intended_balance_sat": -50000, "status": "successful", "subsystem": {{"kind": "send"}}, "sent_to": [{{"destination": {{"type": "ark", "value": "{JOURNAL_ARK}"}}, "amount_sat": 50000}}], "time": {{"completed_at": "{now}"}}}}]}}"#
                        ),
                    )
                } else {
                    (200, r#"{"history": []}"#.to_string())
                }
            }
            _ => (404, "{}".to_string()),
        }
        })
    }

    fn sends(rec: &Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>) -> usize {
        rec.lock()
            .unwrap()
            .iter()
            .filter(|r| r.route() == "/api/v1/wallet/send")
            .count()
    }

    async fn journal_status(db: &Db) -> Vec<String> {
        db.call(|c| {
            let mut st = c.prepare("SELECT status FROM settlements ORDER BY created_at, rowid")?;
            let rows = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn an_unreadable_send_answer_is_unknown_pauses_every_send_and_alerts_once() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let mode = Arc::new(AtomicUsize::new(0));
        let hist = Arc::new(AtomicBool::new(false));
        let (base, rec) = journal_mock(mode.clone(), hist);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap()
            .contains("may or may not"));
        assert_eq!(journal_status(&db).await, vec!["unknown"]);
        assert_eq!(sends(&rec), 1);
        // The send would work now, but nothing is sent while the outcome is unclear:
        // not by the timer, not by the button.
        mode.store(2, Ordering::SeqCst);
        engine.tick().await.unwrap();
        let report = engine.sweep_now().await.unwrap();
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap()
            .contains("may or may not"));
        assert_eq!(sends(&rec), 1, "an unknown send is never retried");
        assert!(db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
        let notices = db
            .call(|c| Ok(crate::settings::get(c, "alert_notices")?.unwrap_or_default()))
            .await
            .unwrap();
        assert_eq!(notices, "transfer_unknown");
    }

    #[tokio::test]
    async fn the_owner_saying_it_did_not_arrive_lets_sending_resume() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let mode = Arc::new(AtomicUsize::new(0));
        let (base, rec) = journal_mock(mode.clone(), Arc::new(AtomicBool::new(false)));
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        engine.tick().await.unwrap();
        mode.store(2, Ordering::SeqCst);
        assert!(db.call(|c| resolve_unsettled(c, false)).await.unwrap());
        assert!(
            !db.call(|c| resolve_unsettled(c, false)).await.unwrap(),
            "nothing left to settle"
        );
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(50_000));
        assert_eq!(sends(&rec), 2);
        assert_eq!(journal_status(&db).await, vec!["failed", "broadcast"]);
    }

    #[tokio::test]
    async fn the_owner_saying_it_arrived_settles_it_as_sent() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let mode = Arc::new(AtomicUsize::new(0));
        let (base, _rec) = journal_mock(mode, Arc::new(AtomicBool::new(false)));
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        engine.tick().await.unwrap();
        assert!(db.call(|c| resolve_unsettled(c, true)).await.unwrap());
        assert_eq!(journal_status(&db).await, vec!["broadcast"]);
        assert!(!db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
    }

    #[tokio::test]
    async fn one_matching_movement_in_the_wallet_history_settles_it_without_the_owner() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let mode = Arc::new(AtomicUsize::new(0));
        let hist = Arc::new(AtomicBool::new(false));
        let (base, _rec) = journal_mock(mode.clone(), hist.clone());
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        engine.tick().await.unwrap();
        assert_eq!(journal_status(&db).await, vec!["unknown"]);
        // The wallet's history now shows the send going out.
        hist.store(true, Ordering::SeqCst);
        mode.store(1, Ordering::SeqCst); // any new send would be refused: only the settling is under test
        engine.tick().await.unwrap();
        let first: (String, String) = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT status, detail FROM settlements ORDER BY created_at, rowid LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(first.0, "broadcast");
        assert!(first.1.contains("wallet history"), "{}", first.1);
        assert!(!db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
    }

    #[tokio::test]
    async fn a_clear_refusal_is_a_plain_failure_and_leaves_no_unclear_line() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let mode = Arc::new(AtomicUsize::new(1));
        let (base, rec) = journal_mock(mode, Arc::new(AtomicBool::new(false)));
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert_eq!(
            journal_status(&db).await,
            vec!["failed"],
            "one line, no leftovers"
        );
        assert_eq!(sends(&rec), 2, "a clear no is retried as before");
        assert!(!db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
    }

    #[tokio::test]
    async fn a_sending_line_left_by_a_restart_pauses_sends_until_settled() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let mode = Arc::new(AtomicUsize::new(2));
        let (base, rec) = journal_mock(mode, Arc::new(AtomicBool::new(false)));
        let db = Db::open(":memory:").unwrap();
        db.call(|c| {
            record_id(c, "sweep_ark", 50_000, JOURNAL_ARK, "sending", Some("{}")).map(|_| ())
        })
        .await
        .unwrap();
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        let report = engine.tick().await.unwrap();
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap()
            .contains("may or may not"));
        assert_eq!(sends(&rec), 0);
        assert_eq!(journal_status(&db).await, vec!["unknown"]);
    }

    #[tokio::test]
    async fn an_onchain_send_whose_coins_are_all_still_free_is_settled_as_not_sent() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let mode = Arc::new(AtomicUsize::new(2));
        let (base, _rec) = journal_mock(mode, Arc::new(AtomicBool::new(false)));
        let db = Db::open(":memory:").unwrap();
        // An attempt over 2 hours ago that named coin v1, which is still free.
        db.call(|c| {
            let id = record_id(
                c,
                "sweep_onchain",
                50_000,
                "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
                "unknown",
                Some(r#"{"vtxos": ["v1"]}"#),
            )?;
            c.execute(
                "UPDATE settlements SET created_at = ?1 WHERE id = ?2",
                params![now_unix() - 8_000, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        engine.tick().await.unwrap();
        let first: (String, String) = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT status, detail FROM settlements ORDER BY created_at, rowid LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(first.0, "failed");
        assert!(first.1.contains("still in the wallet"), "{}", first.1);
    }

    /// An old on-chain attempt that named coin v1, which is still free, against a
    /// wallet whose rounds list and history answer as given.
    async fn old_onchain_attempt_settles_to(
        rounds: (u16, &'static str),
        history: &'static str,
    ) -> String {
        let (base, _rec) = mock::spawn(move |req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 50000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 50000}"#.to_string()),
            "/api/v1/wallet/rounds" => (rounds.0, rounds.1.to_string()),
            r if r.contains("history") => (200, history.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let db = Db::open(":memory:").unwrap();
        db.call(|c| {
            let id = record_id(
                c,
                "sweep_onchain",
                50_000,
                "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
                "unknown",
                Some(r#"{"vtxos": ["v1"]}"#),
            )?;
            c.execute(
                "UPDATE settlements SET created_at = ?1 WHERE id = ?2",
                params![now_unix() - 8_000, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        engine.tick().await.unwrap();
        db.call(|c| {
            Ok(c.query_row(
                "SELECT status FROM settlements ORDER BY created_at, rowid LIMIT 1",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn not_sent_is_never_concluded_without_proof_from_the_wallet() {
        let empty = r#"{"history": []}"#;
        // Proof: rounds readable and empty, no trace in the history.
        assert_eq!(
            old_onchain_attempt_settles_to((200, "[]"), empty).await,
            "failed"
        );
        // The rounds list cannot be read: stays unclear for the owner.
        assert_eq!(
            old_onchain_attempt_settles_to((500, "{}"), empty).await,
            "unknown"
        );
        // The coin is still waiting in a round: stays unclear.
        assert_eq!(
            old_onchain_attempt_settles_to(
                (
                    200,
                    r#"[{"state": "pending", "participation": {"inputs": ["v1"]}}]"#
                ),
                empty
            )
            .await,
            "unknown"
        );
        // An active participation whose coins cannot be read: stays unclear.
        assert_eq!(
            old_onchain_attempt_settles_to((200, r#"[{"state": "ongoing"}]"#), empty).await,
            "unknown"
        );
        // A pending movement to the address in the history: stays unclear.
        assert_eq!(
            old_onchain_attempt_settles_to(
                (200, "[]"),
                r#"{"history": [{"intended_balance_sat": -50000, "status": "pending", "sent_to": [{"destination": {"type": "bitcoin", "value": "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"}, "amount_sat": 50000}]}]}"#
            )
            .await,
            "unknown"
        );
    }

    #[test]
    fn a_destination_matches_only_as_a_whole_address() {
        let e = history_entry(serde_json::json!({
            "sent_to": [{"destination": {"type": "bitcoin", "value": "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"}, "amount_sat": 1}]
        }));
        assert!(e.sent_to_contains("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"));
        assert!(e.sent_to_contains("TB1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KXPJZSX"));
        assert!(!e.sent_to_contains("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzs"));
        assert!(!e.sent_to_contains("bitcoin"));
        assert!(!e.sent_to_contains(""));
    }

    fn history_entry(json: serde_json::Value) -> crate::wallet::barkd::HistoryEntry {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn history_matching_wants_a_successful_recent_send_to_the_same_address() {
        let now = now_unix();
        let u = UnsettledSend {
            id: "stl_1".into(),
            kind: "sweep_ark".into(),
            amount_sats: 50_000,
            destination: "tark1dest".into(),
            created_at: now - 60,
            vtxos: vec![],
        };
        let entry = |status: &str, amount: i64, dest: &str, at: i64| {
            history_entry(serde_json::json!({
                "intended_balance_sat": -amount,
                "status": status,
                "sent_to": [{"destination": {"type": "ark", "value": dest}, "amount_sat": amount}],
                "time": {"completed_at": chrono::DateTime::from_timestamp(at, 0).unwrap().to_rfc3339()},
            }))
        };
        let good = entry("successful", 50_000, "tark1dest", now - 30);
        assert_eq!(
            count_history_matches(&u, std::slice::from_ref(&good), now),
            1
        );
        // A failed or cancelled movement is not our send.
        assert_eq!(
            count_history_matches(&u, &[entry("failed", 50_000, "tark1dest", now - 30)], now),
            0
        );
        // Another address, another amount, or an older send does not match.
        assert_eq!(
            count_history_matches(
                &u,
                &[entry("successful", 50_000, "tark1other", now - 30)],
                now
            ),
            0
        );
        assert_eq!(
            count_history_matches(
                &u,
                &[entry("successful", 20_000, "tark1dest", now - 30)],
                now
            ),
            0
        );
        assert_eq!(
            count_history_matches(
                &u,
                &[entry("successful", 50_000, "tark1dest", now - 4_000)],
                now
            ),
            0
        );
        // Two matches cannot be told apart: not settled by itself.
        assert_eq!(count_history_matches(&u, &[good.clone(), good], now), 2);
    }

    #[tokio::test]
    async fn an_unsettled_send_is_noticed_even_when_the_till_is_now_empty() {
        // The send took everything before the restart: the balance is 0, and the
        // line must still be shown as unclear and alerted, not forgotten.
        let (engine, db) = ark_engine_holding(0).await;
        db.call(|c| {
            record_id(c, "sweep_ark", 50_000, JOURNAL_ARK, "sending", Some("{}")).map(|_| ())
        })
        .await
        .unwrap();
        let report = engine.tick().await.unwrap();
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap()
            .contains("may or may not"));
        assert_eq!(journal_status(&db).await, vec!["unknown"]);
        let notices = db
            .call(|c| Ok(crate::settings::get(c, "alert_notices")?.unwrap_or_default()))
            .await
            .unwrap();
        assert_eq!(notices, "transfer_unknown");
    }

    #[tokio::test]
    async fn a_send_still_in_flight_is_not_shown_to_the_owner_as_unclear() {
        let db = Db::open(":memory:").unwrap();
        db.call(|c| {
            record_id(c, "sweep_ark", 5_000, JOURNAL_ARK, "sending", Some("{}")).map(|_| ())
        })
        .await
        .unwrap();
        assert!(!db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
        assert!(!db.call(|c| resolve_unsettled(c, false)).await.unwrap());
        db.call(|c| {
            c.execute("UPDATE settlements SET created_at = created_at - 300", [])?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(db.call(|c| Ok(transfer_unknown_showing(c))).await.unwrap());
    }

    #[tokio::test]
    async fn did_not_arrive_is_checked_against_the_wallet_history() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let mode = Arc::new(AtomicUsize::new(0));
        let hist = Arc::new(AtomicBool::new(false));
        let (base, _rec) = journal_mock(mode, hist.clone());
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        // Nothing unclear: nothing to check.
        assert_eq!(
            unsettled_shows_in_history(&client, &db).await.unwrap(),
            None
        );
        // An unclear transfer, and the history does not show it.
        let engine = engine_for(&base, &db, JOURNAL_ARK);
        engine.tick().await.unwrap();
        assert_eq!(
            unsettled_shows_in_history(&client, &db).await.unwrap(),
            Some(false)
        );
        // The history shows it: "did not arrive" would be wrong.
        hist.store(true, Ordering::SeqCst);
        assert_eq!(
            unsettled_shows_in_history(&client, &db).await.unwrap(),
            Some(true)
        );
        // History that cannot be read is an error, never "not shown".
        let (broken, _r) = mock::spawn(|_| (500, "{}".to_string()));
        let broken_client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&broken, "t"),
        )
        .unwrap();
        assert!(unsettled_shows_in_history(&broken_client, &db)
            .await
            .is_err());
    }

    #[test]
    fn did_not_arrive_is_refused_for_a_pending_or_odd_amount_send() {
        let now = now_unix();
        let u = UnsettledSend {
            id: "stl_1".into(),
            kind: "sweep_onchain".into(),
            amount_sats: 50_000,
            destination: "tark1dest".into(),
            created_at: now - 60,
            vtxos: vec![],
        };
        let entry = |status: &str, amount: i64, dest: &str, at: i64| {
            history_entry(serde_json::json!({
                "intended_balance_sat": -amount,
                "status": status,
                "sent_to": [{"destination": {"type": "ark", "value": dest}, "amount_sat": amount}],
                "time": {"created_at": chrono::DateTime::from_timestamp(at, 0).unwrap().to_rfc3339()},
            }))
        };
        // Still on its way: shows.
        assert!(history_may_show_send(
            &u,
            &[entry("pending", 50_000, "tark1dest", now - 30)]
        ));
        // Sent, but for a different amount than expected (fees): shows.
        assert!(history_may_show_send(
            &u,
            &[entry("successful", 31_000, "tark1dest", now - 30)]
        ));
        assert!(history_may_show_send(
            &u,
            &[entry("successful", 50_000, "tark1dest", now - 30)]
        ));
        // A status we do not know yet: shows (never assume it did not go out).
        assert!(history_may_show_send(
            &u,
            &[entry("something_new", 50_000, "tark1dest", now - 30)]
        ));
        // Definitely ended without sending: does not show.
        assert!(!history_may_show_send(
            &u,
            &[entry("failed", 50_000, "tark1dest", now - 30)]
        ));
        assert!(!history_may_show_send(
            &u,
            &[entry("canceled", 50_000, "tark1dest", now - 30)]
        ));
        // Another address, or a send from well before this attempt: does not show.
        assert!(!history_may_show_send(
            &u,
            &[entry("pending", 50_000, "tark1other", now - 30)]
        ));
        assert!(!history_may_show_send(
            &u,
            &[entry("successful", 50_000, "tark1dest", now - 3_600)]
        ));
    }

    #[test]
    fn only_a_possibly_delivered_request_counts_as_unknown() {
        use crate::wallet::WalletError;
        assert!(send_outcome_unknown(&WalletError::Unavailable(
            "barkd request timed out: x".into()
        )));
        assert!(send_outcome_unknown(&WalletError::Unavailable(
            "barkd request failed: reset".into()
        )));
        assert!(send_outcome_unknown(&WalletError::Rejected(
            "unparseable response from barkd: eof".into()
        )));
        assert!(!send_outcome_unknown(&WalletError::Unavailable(
            "barkd connection failed: refused".into()
        )));
        assert!(!send_outcome_unknown(&WalletError::Unavailable(
            "barkd returned HTTP 500".into()
        )));
        // A gateway that gave up says nothing about the send behind it.
        assert!(send_outcome_unknown(&WalletError::Unavailable(
            "barkd returned HTTP 502: bad gateway".into()
        )));
        assert!(send_outcome_unknown(&WalletError::Unavailable(
            "barkd returned HTTP 504".into()
        )));
        assert!(!send_outcome_unknown(&WalletError::Rejected(
            "barkd rejected the request (HTTP 400)".into()
        )));
    }

    /// A mock holding several coins; returns the base URL and the requests.
    fn coins_mock(
        coins: &[(&str, i64)],
    ) -> (
        String,
        Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        let list = coins
            .iter()
            .map(|(id, amt)| {
                format!(
                    r#"{{"id": "{id}", "amount_sat": {amt}, "expires_at": 9999999999, "state": {{"type": "spendable"}}}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let total: i64 = coins.iter().map(|(_, a)| a).sum();
        mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (200, format!("[{list}]")),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 20000, "trusted_spendable_sat": 20000, "confirmed_sat": 20000}"#
                    .to_string(),
            ),
            "/api/v1/wallet/balance" => (200, format!(r#"{{"spendable_sat": {total}}}"#)),
            "/api/v1/fees/onchain" => (
                200,
                r#"{"fast_sat_per_vb": 20, "regular_sat_per_vb": 10, "slow_sat_per_vb": 5}"#
                    .to_string(),
            ),
            "/api/v1/wallet/offboard/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        })
    }

    const KEEP_ONCHAIN: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";

    #[tokio::test]
    async fn an_onchain_payout_sends_the_whole_coin() {
        let (base, _recorded) = coins_mock(&[("big", 150_000)]);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, KEEP_ONCHAIN);
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(150_000));
    }

    #[test]
    fn vtxo_selection_covers_the_excess_largest_first() {
        use crate::wallet::barkd::Vtxo;
        let v = |id: &str, amt: i64| Vtxo {
            id: id.into(),
            amount_sats: amt,
            expiry_height: None,
            expires_at: None,
        };
        let set = vec![v("a", 10), v("b", 70), v("c", 30)];
        assert_eq!(select_vtxos_covering(&set, 60), (vec!["b".to_string()], 70));
        assert_eq!(
            select_vtxos_covering(&set, 90),
            (vec!["b".to_string(), "c".to_string()], 100)
        );
        // Less spendable than asked: take everything, caller sees the total.
        assert_eq!(select_vtxos_covering(&set, 500).1, 110);
        assert!(select_vtxos_covering(&[], 5).0.is_empty());
    }

    /// Mock for the fee tests: one 150,000-sat VTXO, NO on-chain reserve,
    /// and a network fee rate of `regular` sat/vB.
    fn fee_mock(
        regular: i64,
        till: i64,
    ) -> (
        String,
        Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": {till}, "expires_at": 9999999999, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/wallet/balance" => (200, format!(r#"{{"spendable_sat": {till}}}"#)),
            "/api/v1/fees/onchain" => (
                200,
                format!(
                    r#"{{"fast_sat_per_vb": {}, "regular_sat_per_vb": {regular}, "slow_sat_per_vb": 1}}"#,
                    regular + 10
                ),
            ),
            "/api/v1/wallet/offboard/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        })
    }

    fn engine_for(base: &str, db: &Db, destination: &str) -> SettlementEngine {
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(base, "t"),
        )
        .unwrap();
        test_engine(client, db.clone(), Some(destination))
    }

    #[tokio::test]
    async fn an_automatic_sweep_waits_for_a_minimum_excess_but_the_button_does_not() {
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        // With 1,499 held the automatic sweep waits (under the 1,500 minimum).
        // With 1,500 held it sends. "Move my money now" always sends.
        let engine_holding = |held: i64| async move {
            let (base, _recorded) = mock::spawn(move |req| match req.route() {
                "/api/v1/wallet/vtxos" => (
                    200,
                    format!(
                        r#"[{{"id": "v1", "amount_sat": {held}, "expires_at": 9999999999, "state": {{"type": "spendable"}}}}]"#
                    ),
                ),
                "/api/v1/onchain/balance" => (
                    200,
                    r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string(),
                ),
                "/api/v1/wallet/balance" => (200, format!(r#"{{"spendable_sat": {held}}}"#)),
                "/api/v1/wallet/send" => (200, r#"{"message": "ok"}"#.to_string()),
                _ => (404, "{}".to_string()),
            });
            let db = Db::open(":memory:").unwrap();
            (engine_for(&base, &db, ARK), base)
        };

        let (engine, _base) = engine_holding(1_499).await;
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        assert_eq!(engine.sweep_now().await.unwrap().swept_sats, Some(1_499));

        let (engine, _base) = engine_holding(1_500).await;
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(1_500));
    }

    #[tokio::test]
    async fn automatic_onchain_sweep_waits_for_low_fees() {
        // No on-chain reserve at all, fees 80 > the 50 limit: nothing is sent,
        // one calm "waiting" row is written (not one per tick).
        let (base, recorded) = fee_mock(80, 150_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        for _ in 0..3 {
            let report = engine.tick().await.unwrap();
            assert!(report.swept_sats.is_none());
            assert!(report.sweep_refused.unwrap().contains("fees"));
        }
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/offboard/vtxos"));
        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE status = 'waiting' AND kind = 'sweep_onchain'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn an_automatic_onchain_sweep_waits_for_50_000_to_send() {
        const ONCHAIN: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        // Nothing kept, fees low. Held 49,999: nothing is sent (and nothing is
        // recorded as a refusal). Held 50,000: sent.
        let (base, recorded) = fee_mock(10, 49_999);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, ONCHAIN);
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report.sweep_refused.is_none());
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/offboard/vtxos"));
        // The button still moves whatever is there.
        assert_eq!(engine.sweep_now().await.unwrap().swept_sats, Some(49_999));

        let (base, _recorded) = fee_mock(10, 50_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, ONCHAIN);
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(50_000));
    }

    #[tokio::test]
    async fn the_owners_onchain_minimum_replaces_the_default() {
        const ONCHAIN: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
        // A chosen minimum of 80,000: held 79,999 waits, held 80,000 goes.
        let (base, _recorded) = fee_mock(10, 79_999);
        let db = Db::open(":memory:").unwrap();
        db.call(|c| crate::payout::set_min(c, crate::payout::Rail::Onchain, 80_000))
            .await
            .unwrap();
        let engine = engine_for(&base, &db, ONCHAIN);
        assert!(engine.tick().await.unwrap().swept_sats.is_none());

        let (base, _recorded) = fee_mock(10, 80_000);
        let db = Db::open(":memory:").unwrap();
        db.call(|c| crate::payout::set_min(c, crate::payout::Rail::Onchain, 80_000))
            .await
            .unwrap();
        let engine = engine_for(&base, &db, ONCHAIN);
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(80_000));
    }

    /// An engine holding `held` sats in one Ark coin, paying out to an Ark
    /// address.
    async fn ark_engine_holding(held: i64) -> (SettlementEngine, Db) {
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        let (base, _recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": {held}, "expires_at": 9999999999, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, format!(r#"{{"spendable_sat": {held}}}"#)),
            "/api/v1/wallet/send" => (200, r#"{"message": "ok"}"#.to_string()),
            _ => (404, "{}".to_string()),
        });
        let db = Db::open(":memory:").unwrap();
        (engine_for(&base, &db, ARK), db)
    }

    #[tokio::test]
    async fn an_ark_sweep_sends_the_whole_balance_once_the_minimum_is_reached() {
        // Keeps nothing: 1,499 waits, 1,500 sends all 1,500, 2,345 sends all 2,345.
        let (engine, _db) = ark_engine_holding(1_499).await;
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        let (engine, _db) = ark_engine_holding(1_500).await;
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(1_500));
        let (engine, _db) = ark_engine_holding(2_345).await;
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(2_345));
    }

    #[tokio::test]
    async fn the_owners_ark_minimum_replaces_the_default() {
        let (engine, db) = ark_engine_holding(2_999).await;
        db.call(|c| crate::payout::set_min(c, crate::payout::Rail::Ark, 3_000))
            .await
            .unwrap();
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        let (engine, db) = ark_engine_holding(3_000).await;
        db.call(|c| crate::payout::set_min(c, crate::payout::Rail::Ark, 3_000))
            .await
            .unwrap();
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(3_000));
    }

    #[tokio::test]
    async fn coins_locked_in_a_renewal_round_are_never_swept() {
        // Two coins: "held" is in a round the wallet is still working on (barkd
        // can still list it as spendable), "free" is not. Both far from expiry.
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        let later = now_unix() + 20 * 86_400;
        let vtxos = format!(
            r#"[{{"id": "held", "amount_sat": 5000, "expires_at": {later}, "state": {{"type": "spendable"}}}},
                {{"id": "free", "amount_sat": 3000, "expires_at": {later}, "state": {{"type": "spendable"}}}}]"#
        );
        let (base, recorded) = mock::spawn(move |req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (200, vtxos.clone()),
            "/api/v1/wallet/rounds" => (
                200,
                r#"[{"id": 9, "state": "ongoing", "participation": {"inputs": ["held"], "outputs": []}}]"#
                    .to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 8000}"#.to_string()),
            "/api/v1/wallet/send" => (200, r#"{"message": "ok"}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(client, db.clone(), Some(ARK));
        // Automatic: waits for the round, quietly (nothing sent, nothing failed).
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report.sweep_refused.is_none());
        let failed: i64 = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE status = 'failed'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(failed, 0);
        // The owner's button sends only the free coins' worth.
        assert_eq!(engine.sweep_now().await.unwrap().swept_sats, Some(3_000));
        let sent: Vec<serde_json::Value> = recorded
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.route() == "/api/v1/wallet/send")
            .map(|r| serde_json::from_str(&r.body).unwrap())
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["amount_sat"], 3_000);
    }

    #[tokio::test]
    async fn a_renewal_asks_for_at_most_1000_coins_soonest_expiring_first() {
        let soon = crate::util::now_unix() + 86_400;
        // 1,200 coins, all due; coin vN expires N seconds after the first.
        let coins: Vec<String> = (0..1_200)
            .map(|i| {
                format!(
                    r#"{{"id": "v{i}", "amount_sat": 1000, "expires_at": {}, "state": {{"type": "spendable"}}}}"#,
                    soon + i
                )
            })
            .collect();
        let vtxos = format!("[{}]", coins.join(","));
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (200, vtxos.clone()),
            "/api/v1/wallet/balance" => (500, "{}".to_string()),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let engine = test_engine(client, Db::open(":memory:").unwrap(), None);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1_000);
        let requests = recorded.lock().unwrap();
        let refresh = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/refresh/vtxos")
            .expect("refresh recorded");
        let body: serde_json::Value = serde_json::from_str(&refresh.body).unwrap();
        let ids: Vec<&str> = body["vtxos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 1_000);
        assert!(ids.contains(&"v0") && ids.contains(&"v999"));
        assert!(!ids.contains(&"v1000") && !ids.contains(&"v1199"));
    }

    #[tokio::test]
    async fn automatic_onchain_sweep_goes_when_fees_are_low_even_without_a_reserve() {
        let (base, recorded) = fee_mock(50, 150_000); // exactly at the limit
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/offboard/vtxos"));
    }

    #[tokio::test]
    async fn manual_sweep_ignores_the_fee_limit() {
        // Fees 200 (over the 50 limit) and a till of 120,000: the automatic path
        // waits, the button moves everything (the fee, about 80,000, still leaves plenty).
        let (base, recorded) = fee_mock(200, 120_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        let report = engine.sweep_now().await.unwrap();
        assert_eq!(report.swept_sats, Some(120_000));
        let body = {
            let requests = recorded.lock().unwrap();
            let offboard = requests
                .iter()
                .find(|r| r.route() == "/api/v1/wallet/offboard/vtxos")
                .expect("manual sweep must offboard");
            serde_json::from_str::<serde_json::Value>(&offboard.body).unwrap()
        };
        assert_eq!(
            body["address"],
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        );
    }

    #[test]
    fn onchain_arrival_check_holds_back_sends_the_fee_would_eat() {
        // 10 sat/vB x 400 vB = 4,000 sats of estimated fee.
        assert!(onchain_arrival_problem(150_000, 10).is_none());
        assert!(
            onchain_arrival_problem(5_000, 10).is_none(),
            "exactly 1,000 arrives"
        );
        let low = onchain_arrival_problem(4_999, 10).expect("999 would arrive");
        assert!(low.contains("Only about 999 sats would arrive"), "{low}");
        assert!(low.contains("nothing was sent"), "{low}");
        let eaten = onchain_arrival_problem(5_000, 200).expect("fee is more than the amount");
        assert!(eaten.contains("is more than the 5000 sats"), "{eaten}");
        assert!(
            onchain_arrival_problem(0, 0).is_some(),
            "nothing is not worth sending"
        );
    }

    #[tokio::test]
    async fn the_button_holds_back_an_onchain_send_the_fee_would_eat() {
        // 4,000 sats in the till, fees 10 sat/vB (about 4,000 sats estimated):
        // nothing would arrive, so nothing is sent and the reason is shown.
        let (base, recorded) = fee_mock(10, 4_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        for _ in 0..2 {
            let report = engine.sweep_now().await.unwrap();
            assert!(report.swept_sats.is_none());
            let why = report.sweep_refused.expect("a plain reason");
            assert!(why.contains("nothing was sent"), "{why}");
        }
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/offboard/vtxos"));
        // Written once, not once per press.
        let rows: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE status = 'waiting' AND kind = 'sweep_onchain'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn the_button_holds_back_an_ark_send_under_330_sats() {
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 300, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 300}"#.to_string()),
            "/api/v1/wallet/send" => (200, r#"{"message": "ok"}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, ARK);
        let report = engine.sweep_now().await.unwrap();
        assert!(report.swept_sats.is_none());
        let why = report.sweep_refused.expect("a plain reason");
        assert!(why.contains("too small to send"), "{why}");
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
    }

    #[test]
    fn upgrade_errors_are_recognised_and_ordinary_ones_are_not() {
        assert!(is_update_needed_error(
            "barkd rejected the request (HTTP 400: Please upgrade your wallet to spend this coin)"
        ));
        assert!(is_update_needed_error("client version is too old"));
        assert!(!is_update_needed_error("insufficient funds"));
        assert!(!is_update_needed_error(
            "barkd returned HTTP 500: problem A"
        ));
    }

    #[tokio::test]
    async fn a_refusal_for_old_wallet_software_pauses_the_automatic_transfer_with_one_alert() {
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 5000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 5000}"#.to_string()),
            "/api/v1/wallet/send" => (
                400,
                r#"{"message": "Please upgrade your wallet to spend this coin"}"#.to_string(),
            ),
            _ => (404, "{}".to_string()),
        }
        });
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, ARK);
        let sends = || {
            recorded
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.route() == "/api/v1/wallet/send")
                .count()
        };
        // The first (automatic) try is refused as too old.
        engine.tick().await.unwrap();
        assert_eq!(sends(), 1);
        let (showing, paused, notices) = db
            .call(|c| {
                Ok((
                    update_needed_showing(c, now_unix()),
                    update_needed_paused(c, now_unix()),
                    crate::settings::get(c, "alert_notices")?.unwrap_or_default(),
                ))
            })
            .await
            .unwrap();
        assert!(showing && paused);
        assert_eq!(notices, "wallet_update_needed");
        // The next automatic checks do not ask again.
        engine.tick().await.unwrap();
        engine.tick().await.unwrap();
        assert_eq!(sends(), 1, "the automatic transfer must stop asking");
        // The button still tries (and is refused again, with no second alert).
        engine.sweep_now().await.unwrap();
        assert_eq!(sends(), 2);
        let notices = db
            .call(|c| Ok(crate::settings::get(c, "alert_notices")?.unwrap_or_default()))
            .await
            .unwrap();
        assert_eq!(notices, "wallet_update_needed");
        // The failed line says what is wrong in plain words.
        let detail: String = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT detail FROM settlements WHERE status = 'failed' LIMIT 1",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert!(
            detail.starts_with("Your wallet software is too old"),
            "{detail}"
        );
    }

    #[tokio::test]
    async fn unreadable_fee_rates_postpone_an_automatic_onchain_sweep() {
        let (base, _recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 150000, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(&base, &db, "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx");
        let report = engine.tick().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report.sweep_refused.unwrap().contains("fee rates"));
    }

    #[tokio::test]
    async fn ark_sweep_does_not_need_the_exit_reserve() {
        // Same shape as the refused on-chain case (two VTXOs, 0 on-chain
        // sats) but the payout is an Ark address: it must go through.
        let (base, recorded) = mock::spawn(|req| {
            match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                r#"[{"id": "v1", "amount_sat": 200000, "expires_at": 9999999999, "state": {"type": "spendable"}}, {"id": "v2", "amount_sat": 1, "expires_at": 9999999999, "state": {"type": "spendable"}}]"#.to_string(),
            ),
            "/api/v1/onchain/balance" => (200, r#"{"total_sat": 0, "trusted_spendable_sat": 0}"#.to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 200001}"#.to_string()),
            "/api/v1/wallet/send" => (200, r#"{"message": "ok"}"#.to_string()),
            _ => (404, "{}".to_string()),
        }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(200_001));
        assert!(report.sweep_refused.is_none());
        let sent = recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send");
        assert!(sent, "the Ark send must be made");
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
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 15000, "trusted_spendable_sat": 15000}"#.to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 1000}"#.to_string()),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 1);
        assert!(report.swept_sats.is_none(), "below the minimum: no sweep");

        let requests = recorded.lock().unwrap();
        let refresh = requests
            .iter()
            .find(|r| r.route() == "/api/v1/wallet/refresh/vtxos")
            .expect("refresh recorded");
        assert_eq!(refresh.method, "POST");
        let body: serde_json::Value = serde_json::from_str(&refresh.body).unwrap();
        assert_eq!(body["vtxos"][0], "v1");
    }

    #[tokio::test]
    async fn without_a_destination_renewal_still_runs_and_sweeping_stays_off() {
        // A missing sweep destination disables SWEEPING, never renewal.
        let (base, recorded) = mock::spawn(|req| {
            if req.route().contains("/wallet/vtxos") {
                (
                    200,
                    format!(
                        r#"[{{"id": "v1", "amount_sat": 50000, "state": {{"type": "spendable"}}, "expiry_timestamp": {}}}]"#,
                        crate::util::now_unix() + 3_600
                    ),
                )
            } else if req.route().contains("/onchain/balance") {
                (200, r#"{"confirmed_sat": 30000}"#.to_string())
            } else if req.route().contains("/wallet/balance") {
                (
                    200,
                    r#"{"spendable_sat": 0, "pending_incoming_sat": 0, "claimable_lightning_receive_sat": 0}"#
                        .to_string(),
                )
            } else {
                (200, "{}".to_string()) // refresh
            }
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(client, db, None);
        let report = engine.tick().await.unwrap();
        assert_eq!(
            report.refreshed_vtxos, 1,
            "renewal must run without a destination"
        );
        assert!(report.swept_sats.is_none());
        assert!(report.sweep_refused.is_none());
        let all = recorded.lock().unwrap();
        assert!(
            all.iter()
                .all(|r| r.method == "GET" || r.route().contains("refresh")),
            "no sweep may be sent without a destination"
        );
    }

    /// Spawn a mock barkd whose single VTXO reports a block-HEIGHT expiry
    /// (the real barkd 0.8.0 shape — no `expires_at`) against a tip of 800.
    fn height_expiry_engine(
        expiry_height: u64,
    ) -> (
        SettlementEngine,
        Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        let now = crate::util::now_unix();
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 1000, "expiry_height": {expiry_height}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/bitcoin/tip" => (200, format!(r#"{{"height": 800, "timestamp": {now}}}"#)),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 15000, "trusted_spendable_sat": 15000, "confirmed_sat": 15000}"#
                    .to_string(),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 1000}"#.to_string()),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let engine = test_engine(client, Db::open(":memory:").unwrap(), None);
        (engine, recorded)
    }

    #[tokio::test]
    async fn height_expiry_far_away_is_not_refreshed() {
        // 800 + 4_000 blocks ≈ 28 days out; lead is 7 days (~1_008 blocks).
        let (engine, _rec) = height_expiry_engine(4_800);
        let report = engine.tick().await.unwrap();
        assert_eq!(
            report.refreshed_vtxos, 0,
            "regression: heights were treated as always-due"
        );
    }

    #[tokio::test]
    async fn height_expiry_inside_the_window_is_refreshed() {
        // 800 + 100 blocks ≈ 17 hours out: inside the 7-day lead.
        let (engine, _rec) = height_expiry_engine(900);
        let report = engine.tick().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 1);
    }

    #[test]
    fn refresh_trigger_follows_the_servers_terms() {
        use crate::wallet::barkd::ArkTerms;
        let terms = |life, free| {
            Some(ArkTerms {
                vtxo_lifetime_blocks: life,
                free_refresh_below_blocks: free,
                round_interval_secs: None,
            })
        };
        let week = 604_800; // the default lead time: 1_008 blocks
                            // Unknown terms: exactly the configured lead time (the old rule).
        assert_eq!(refresh_trigger_blocks(week, None), 1_008);
        assert_eq!(refresh_trigger_blocks(week, terms(None, None)), 1_008);
        // The real signet server: coins live 144 blocks, refresh is free
        // below 288. A third of the lifetime is the limit: 48 blocks.
        assert_eq!(
            refresh_trigger_blocks(week, terms(Some(144), Some(288))),
            48
        );
        // A mainnet-like server: long-lived coins, free in the last two
        // days (288 blocks): wait until inside the free window.
        assert_eq!(
            refresh_trigger_blocks(week, terms(Some(4_320), Some(288))),
            287
        );
        // No free window known: the lead time or a third of the life.
        assert_eq!(
            refresh_trigger_blocks(week, terms(Some(4_320), None)),
            1_008
        );
        // An operator who asks for a shorter lead time keeps it.
        assert_eq!(
            refresh_trigger_blocks(86_400, terms(Some(4_320), Some(288))),
            144
        );
        // Never so late that a failed refresh cannot be retried.
        assert_eq!(refresh_trigger_blocks(week, terms(Some(18), Some(288))), 12);
        assert_eq!(
            refresh_trigger_blocks(60, terms(Some(4_320), Some(288))),
            12
        );
    }

    #[test]
    fn slow_rounds_move_the_renewal_point_earlier() {
        use crate::wallet::barkd::ArkTerms;
        let week = 604_800;
        let mainnet = |interval| {
            Some(ArkTerms {
                vtxo_lifetime_blocks: Some(4_032),
                free_refresh_below_blocks: Some(288),
                round_interval_secs: interval,
            })
        };
        // Mainnet terms: renew at 287 blocks (~48 h). Rounds every 5 min,
        // 1 h and 6 h all leave at least 4 rounds: no change.
        for interval in [Some(300), Some(3_600), Some(21_600), None] {
            assert_eq!(
                refresh_trigger_blocks(week, mainnet(interval)),
                287,
                "{interval:?}"
            );
        }
        // A round a day leaves under 2: renew 4 days (576 blocks) ahead.
        assert_eq!(refresh_trigger_blocks(week, mainnet(Some(86_400))), 576);
        // Never past half a coin's life (signet: 144-block coins).
        let signet = |interval| {
            Some(ArkTerms {
                vtxo_lifetime_blocks: Some(144),
                free_refresh_below_blocks: Some(288),
                round_interval_secs: interval,
            })
        };
        assert_eq!(refresh_trigger_blocks(week, signet(Some(300))), 48);
        assert_eq!(refresh_trigger_blocks(week, signet(Some(86_400))), 72);
    }

    /// Mock barkd shaped like the signet drill: tip at 800, one 1_000-sat
    /// coin expiring at `expiry_height`, optionally the real ark-info terms
    /// (144-block coins, refresh free below 288 blocks) and a wallet that
    /// reports `needs_refresh` sats.
    fn signet_engine(expiry_height: u64, with_terms: bool, needs_refresh: i64) -> SettlementEngine {
        let now = crate::util::now_unix();
        let (base, _recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 1000, "expiry_height": {expiry_height}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/wallet/ark-info" if with_terms => (
                200,
                r#"{"vtxo_lifetime":144,"fees":{"refresh":{"base_fee_sat":0,"ppm_expiry_table":[
                    {"expiry_blocks_threshold":0,"ppm":0},
                    {"expiry_blocks_threshold":288,"ppm":2000},
                    {"expiry_blocks_threshold":1008,"ppm":4000}]}}}"#
                    .to_string(),
            ),
            "/api/v1/bitcoin/tip" => (200, format!(r#"{{"height": 800, "timestamp": {now}}}"#)),
            "/api/v1/onchain/balance" => (
                200,
                r#"{"total_sat": 15000, "trusted_spendable_sat": 15000, "confirmed_sat": 15000}"#
                    .to_string(),
            ),
            "/api/v1/wallet/balance" => (
                200,
                format!(r#"{{"spendable_sat": 1000, "needs_refresh_sat": {needs_refresh}}}"#),
            ),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        test_engine(client, Db::open(":memory:").unwrap(), None)
    }

    #[tokio::test]
    async fn a_fresh_short_lived_coin_is_left_alone_so_money_is_not_stuck_in_refreshes() {
        // 143 blocks left of 144: brand new. The old 7-day rule refreshed
        // it every tick, forever (the droplet test, 8 Nov 2026).
        let engine = signet_engine(800 + 143, true, 0);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
    }

    #[tokio::test]
    async fn a_short_lived_coin_is_refreshed_when_it_gets_near_the_end_of_its_life() {
        // 40 blocks left: inside the last third (48).
        let engine = signet_engine(800 + 40, true, 0);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
    }

    #[tokio::test]
    async fn a_coin_that_arrives_nearly_expired_is_refreshed_straight_away() {
        // Coins paid over Ark keep the sender's expiry: 16 blocks left.
        let engine = signet_engine(800 + 16, true, 0);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
    }

    async fn renew_note(engine: &SettlementEngine) -> String {
        engine
            .db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT detail FROM settlements WHERE kind = 'renew' ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_upkeep_note_says_a_coin_was_renewed_because_it_arrived_near_expiry() {
        // 16 blocks left on signet (about 2.7 hours): the note says why.
        let engine = signet_engine(800 + 16, true, 0);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        let note = renew_note(&engine).await;
        assert!(
            note.contains("Ark server's next round. Why: 1 coin is close to expiring"),
            "{note}"
        );
        assert!(
            note.contains("the soonest has about 2 hours left"),
            "{note}"
        );
    }

    #[tokio::test]
    async fn the_upkeep_note_says_when_the_wallet_flagged_the_coins() {
        // 143 blocks left: not due by timing, but inside the free window and
        // the wallet flags it, so it is renewed because the wallet said so.
        let engine = signet_engine(800 + 143, true, 1_000);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        let note = renew_note(&engine).await;
        assert!(
            note.contains("your wallet said 1 more coin must be refreshed before it can be sent"),
            "{note}"
        );
        assert!(!note.contains("close to expiring"), "{note}");
    }

    #[tokio::test]
    async fn a_new_line_carries_the_saved_dollar_price_and_stays_blank_without_one() {
        let db = Db::open(":memory:").unwrap();
        db.call(|conn| {
            let fiat = |id: &str| -> Result<Vec<Option<String>>, AppError> {
                Ok(conn.query_row(
                    "SELECT fiat_currency, fiat_amount, rate, rate_source FROM settlements WHERE id = ?1",
                    [id],
                    |r| Ok(vec![r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]),
                )?)
            };
            // No saved price: blank, not guessed.
            let a = record_id(conn, "sweep_ark", 150_000, "d", "broadcast", None)?;
            assert_eq!(fiat(&a)?, vec![None, None, None, None]);
            // A recent saved price: 150,000 sats at 60,000 USD/BTC = 90.00.
            let json = format!(
                r#"{{"USD":{{"rate":60000.0,"at":{},"sources":["a","b"]}}}}"#,
                now_unix() - 30
            );
            crate::settings::set(conn, "btc_price", &json)?;
            let b = record_id(conn, "sweep_ark", 150_000, "d", "broadcast", None)?;
            assert_eq!(
                fiat(&b)?,
                vec![
                    Some("USD".to_string()),
                    Some("90.00".to_string()),
                    Some("60000.00".to_string()),
                    Some("median(a,b)".to_string())
                ]
            );
            // An old saved price is not used.
            let old = format!(
                r#"{{"USD":{{"rate":60000.0,"at":{},"sources":["a"]}}}}"#,
                now_unix() - crate::rates::MAX_AGE_SECS - 10
            );
            crate::settings::set(conn, "btc_price", &old)?;
            let c = record_id(conn, "renew", 1_000, "", "requested", None)?;
            assert_eq!(fiat(&c)?, vec![None, None, None, None]);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn renewal_reasons_are_plain_and_joined() {
        assert_eq!(
            renew_reason(2, Some(3 * 86_400 + 5), 0, 0),
            "2 coins are close to expiring (the soonest has about 3 days left)"
        );
        assert_eq!(
            renew_reason(1, Some(1_200), 3, 2),
            "1 coin is close to expiring (the soonest has less than an hour left); your wallet said 3 more coins must be refreshed before they can be sent; 2 small coins were added so the total reaches the 330-sat minimum"
        );
        assert_eq!(
            renew_reason(1, None, 0, 0),
            "1 coin is due and the expiry could not be read"
        );
        assert_eq!(renew_reason(0, None, 0, 0), "the wallet asked for them");
        assert_eq!(time_left_words(-5), "less than an hour");
        assert_eq!(time_left_words(3_600), "about 1 hour");
        assert_eq!(time_left_words(47 * 3_600), "about 47 hours");
        assert_eq!(time_left_words(2 * 86_400), "about 2 days");
    }

    #[tokio::test]
    async fn without_the_servers_terms_the_old_lead_time_still_applies() {
        // ark-info unreadable: 143 blocks left is inside the 7-day lead, so
        // it is refreshed (cautious, never misses a renewal).
        let engine = signet_engine(800 + 143, false, 0);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
    }

    #[tokio::test]
    async fn coins_the_wallet_says_need_refreshing_are_refreshed_even_if_our_timing_says_wait() {
        let engine = signet_engine(800 + 143, true, 1_000);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
    }

    #[tokio::test]
    async fn flagged_coins_are_only_topped_up_inside_the_free_window() {
        // 500 blocks left: outside the free window (below 288). The wallet
        // flags sats, but refreshing this coin now would cost a fee.
        let engine = signet_engine(800 + 500, true, 1_000);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
    }

    /// A mock barkd with one coin due for renewal (expires in a day), a
    /// balance well above the minimum, and a refresh answer chosen by the test.
    fn due_and_rich_engine(
        refresh_status: u16,
    ) -> (
        SettlementEngine,
        Db,
        std::sync::Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        let soon = crate::util::now_unix() + 86_400;
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 150000, "expires_at": {soon}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            "/api/v1/wallet/refresh/vtxos" => {
                (refresh_status, r#"{"message": "round failed"}"#.to_string())
            }
            "/api/v1/wallet/send" => (200, r#"{"sent": true}"#.to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        (engine, db, recorded)
    }

    #[tokio::test]
    async fn a_failed_renewal_is_recorded_so_home_can_warn() {
        let (engine, db, recorded) = due_and_rich_engine(500);
        let report = engine.tick().await.unwrap();
        // A failing renewal must never hold withdrawals back: that is when
        // the owner most needs the money to keep moving out.
        assert_eq!(report.swept_sats, Some(150_000));
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
        // ... and "move my money now" works too (whole till).
        assert_eq!(engine.sweep_now().await.unwrap().swept_sats, Some(150_000));
        let failed: i64 = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM settlements WHERE kind = 'renew' AND status = 'failed'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            failed, 2,
            "one per round that tried (the tick and the button)"
        );
    }

    #[tokio::test]
    async fn no_sweep_in_a_cycle_that_started_a_renewal() {
        let (engine, db, recorded) = due_and_rich_engine(200);
        let report = engine.tick().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 1);
        assert!(
            report.swept_sats.is_none(),
            "the balance read may count locked coins"
        );
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
        // The renewal point is saved for the coin-expiry phone alert.
        let saved = db
            .call(|c| crate::settings::get(c, crate::alerts::RENEWAL_TRIGGER_KEY))
            .await
            .unwrap();
        assert_eq!(saved.as_deref(), Some("604800"));
        // The owner's button reads the wallet again after the renewal. The only
        // coin is now waiting in the round (the mock still lists it as
        // spendable, as barkd 0.8.0 does), so nothing is sent and it says why.
        let report = engine.sweep_now().await.unwrap();
        assert_eq!(report.swept_sats, None);
        assert!(report
            .sweep_refused
            .as_deref()
            .unwrap_or("")
            .contains("renewing your coins"));
        // ...and the amount that is locked is reported too, for the button's message.
        assert_eq!(
            report.held_back,
            vec![HeldBack {
                kind: "renewing",
                sats: report.held_back[0].sats
            }],
        );
        assert!(report.held_back[0].sats > 0);
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
    }

    #[test]
    fn held_back_parts_lists_each_reason_once_and_skips_zeros() {
        use crate::wallet::barkd::Balances;
        let none = Balances::default();
        assert!(held_back_parts(0, &none).is_empty());
        // Locked coins the wallet still counts as spendable.
        assert_eq!(
            held_back_parts(900, &none),
            vec![HeldBack {
                kind: "renewing",
                sats: 900
            }]
        );
        // A renewal is not counted twice when the wallet reports it as well.
        let b = Balances {
            pending_in_round_sats: 900,
            ..Default::default()
        };
        assert_eq!(
            held_back_parts(900, &b),
            vec![HeldBack {
                kind: "renewing",
                sats: 900
            }]
        );
        let all = Balances {
            pending_in_round_sats: 0,
            claimable_lightning_sats: 300,
            pending_board_sats: 50,
            ..Default::default()
        };
        assert_eq!(
            held_back_parts(0, &all),
            vec![
                HeldBack {
                    kind: "collecting",
                    sats: 300
                },
                HeldBack {
                    kind: "confirming",
                    sats: 50
                },
            ]
        );
    }

    #[tokio::test]
    async fn renewal_runs_even_when_the_balance_cannot_be_read() {
        let soon = crate::util::now_unix() + 86_400;
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 1000, "expires_at": {soon}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/wallet/balance" => (500, "{}".to_string()),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let engine = test_engine(client, Db::open(":memory:").unwrap(), None);
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/refresh/vtxos"));
    }

    // -- Renewal on mainnet (archived mainnet plan, PR D) ------------------------------

    #[test]
    fn a_requested_renewal_is_judged_from_the_coin_and_round_lists() {
        use std::collections::HashSet;
        let now = 10_000;
        let r = RequestedRenewal {
            row: "stl_1".into(),
            inputs: vec!["a".into(), "b".into()],
            at: now - 600,
        };
        let set = |ids: &[&'static str]| ids.iter().copied().collect::<HashSet<&str>>();
        let none = set(&[]);
        // Still in an active participation: waiting, whatever else is true.
        assert_eq!(
            judge_requested(&r, &set(&["a"]), &set(&["a"]), &none, true, now),
            RenewalOutcome::Waiting
        );
        // The round failed or was canceled.
        assert!(matches!(
            judge_requested(&r, &set(&["a"]), &none, &set(&["b"]), true, now),
            RenewalOutcome::Failed(_)
        ));
        // The old coins are gone: renewed.
        assert_eq!(
            judge_requested(&r, &set(&["z"]), &none, &none, true, now),
            RenewalOutcome::Renewed
        );
        // Coins still there and no round holds them: wait (a round list we
        // misread must never raise a false alarm), then give up after a while.
        assert_eq!(
            judge_requested(&r, &set(&["b"]), &none, &none, true, now),
            RenewalOutcome::Waiting
        );
        let late = RequestedRenewal {
            at: now - REQUESTED_GIVE_UP_SECS,
            ..r.clone()
        };
        assert!(matches!(
            judge_requested(&late, &set(&["b"]), &none, &none, true, now),
            RenewalOutcome::Failed(_)
        ));
        // Round list unreadable: wait, then give up after a while.
        assert_eq!(
            judge_requested(&r, &set(&["b"]), &none, &none, false, now),
            RenewalOutcome::Waiting
        );
        let old = RequestedRenewal {
            at: now - REQUESTED_GIVE_UP_SECS,
            ..r
        };
        assert!(matches!(
            judge_requested(&old, &set(&["b"]), &none, &none, false, now),
            RenewalOutcome::Failed(_)
        ));
    }

    #[test]
    fn coins_already_in_a_round_are_not_a_failure() {
        for msg in [
            "barkd rejected the request (HTTP 400 Bad Request: {\"message\": \"inputs already held by a round attempt\"})",
            "delegated round participation would replace one a round attempt holds",
            "vtxo is locked",
        ] {
            assert!(is_already_in_round(msg), "{msg}");
        }
        for msg in [
            "barkd returned HTTP 500: {\"message\": \"round failed\"}",
            "wallet unavailable: connection refused",
            "the wallet is locked", // not about coins or rounds
            "",
        ] {
            assert!(!is_already_in_round(msg), "{msg}");
        }
    }

    /// A mock wallet for renewal tests whose coin list and round list can be
    /// changed between cycles. No payout address, so nothing is swept.
    fn renewal_engine(
        vtxos: Arc<std::sync::Mutex<String>>,
        rounds: Arc<std::sync::Mutex<(u16, String)>>,
        refresh: (u16, &'static str),
    ) -> (
        SettlementEngine,
        Db,
        Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) {
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/wallet/vtxos" => (200, vtxos.lock().unwrap().clone()),
            "/api/v1/wallet/rounds" => rounds.lock().unwrap().clone(),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 0}"#.to_string()),
            "/api/v1/wallet/refresh/vtxos" => (refresh.0, refresh.1.to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(client, db.clone(), None);
        (engine, db, recorded)
    }

    fn coins(list: &[(&str, i64, i64)]) -> String {
        let items: Vec<String> = list
            .iter()
            .map(|(id, sats, expires_at)| {
                format!(
                    r#"{{"id": "{id}", "amount_sat": {sats}, "expires_at": {expires_at}, "state": {{"type": "spendable"}}}}"#
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }

    async fn renew_rows(db: &Db) -> Vec<(String, i64)> {
        db.call(|c| {
            let mut st = c.prepare(
                "SELECT status, amount_sats FROM settlements WHERE kind = 'renew' ORDER BY rowid",
            )?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    fn refresh_calls(
        rec: &Arc<std::sync::Mutex<Vec<crate::wallet::barkd::mock::Recorded>>>,
    ) -> Vec<String> {
        rec.lock()
            .unwrap()
            .iter()
            .filter(|r| r.route() == "/api/v1/wallet/refresh/vtxos")
            .map(|r| r.body.clone())
            .collect()
    }

    #[tokio::test]
    async fn coins_too_small_to_renew_are_skipped_quietly_and_shown_on_home() {
        let soon = now_unix() + 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("tiny", 136, soon)])));
        let rounds = Arc::new(std::sync::Mutex::new((200, "[]".to_string())));
        let (engine, db, rec) = renewal_engine(vtxos, rounds, (200, "{}"));
        for _ in 0..3 {
            assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        }
        assert!(
            refresh_calls(&rec).is_empty(),
            "never asks for under 330 sats"
        );
        assert!(
            renew_rows(&db).await.is_empty(),
            "nothing recorded, so no warning"
        );
        let (sats, lapse) = db
            .call(|c| {
                Ok((
                    crate::settings::get(c, TOO_SMALL_SATS_KEY)?,
                    crate::settings::get(c, TOO_SMALL_LAPSE_AT_KEY)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(sats.as_deref(), Some("136"));
        assert_eq!(lapse, Some(soon.to_string()));
    }

    #[tokio::test]
    async fn a_small_due_coin_is_renewed_together_with_the_others() {
        let soon = now_unix() + 86_400;
        let later = now_unix() + 20 * 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[
            ("tiny", 136, soon),
            ("big", 1_000, later),
        ])));
        let rounds = Arc::new(std::sync::Mutex::new((200, "[]".to_string())));
        let (engine, db, rec) = renewal_engine(vtxos, rounds, (200, "{}"));
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 2);
        let calls = refresh_calls(&rec);
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].contains("tiny") && calls[0].contains("big"),
            "{}",
            calls[0]
        );
        assert_eq!(
            renew_rows(&db).await,
            vec![("requested".to_string(), 1_136)]
        );
        let sats = db
            .call(|c| crate::settings::get(c, TOO_SMALL_SATS_KEY))
            .await
            .unwrap();
        assert_eq!(sats, None);
    }

    #[tokio::test]
    async fn a_renewal_says_requested_until_the_old_coins_are_replaced() {
        let soon = now_unix() + 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("c1", 5_000, soon)])));
        let rounds = Arc::new(std::sync::Mutex::new((200, "[]".to_string())));
        let (engine, db, rec) = renewal_engine(vtxos.clone(), rounds.clone(), (200, "{}"));
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        assert_eq!(
            renew_rows(&db).await,
            vec![("requested".to_string(), 5_000)]
        );

        // Next cycle: barkd still lists the coin as spendable while it waits
        // for the round (0.8.0), and the round list holds it. Not asked again.
        *rounds.lock().unwrap() = (
            200,
            r#"[{"id": 3, "state": "pending", "status": {"status": "pending"},
                 "participation": {"inputs": ["c1"], "outputs": []},
                 "unlock_hash": null, "funding_txid": null}]"#
                .to_string(),
        );
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        assert_eq!(refresh_calls(&rec).len(), 1, "never asks twice for c1");
        assert_eq!(
            renew_rows(&db).await,
            vec![("requested".to_string(), 5_000)]
        );

        // The round is done: c1 is replaced by a fresh 28-day coin.
        let fresh = now_unix() + 28 * 86_400;
        *vtxos.lock().unwrap() = coins(&[("c2", 5_000, fresh)]);
        *rounds.lock().unwrap() = (200, "[]".to_string());
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        assert_eq!(renew_rows(&db).await, vec![("renewed".to_string(), 5_000)]);
    }

    #[tokio::test]
    async fn coins_held_by_a_round_attempt_are_never_requested_again() {
        // Due (inside the 7-day lead) but not yet close to expiry.
        let soon = now_unix() + 4 * 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("c1", 5_000, soon)])));
        // Requested earlier (another process, or before a restart).
        let rounds = Arc::new(std::sync::Mutex::new((
            200,
            r#"[{"id": 9, "state": "ongoing", "participation": {"inputs": ["c1"], "outputs": []}}]"#
                .to_string(),
        )));
        let (engine, db, rec) = renewal_engine(vtxos, rounds, (200, "{}"));
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        assert!(refresh_calls(&rec).is_empty());
        assert!(renew_rows(&db).await.is_empty());
    }

    #[tokio::test]
    async fn a_coin_close_to_expiry_is_asked_for_again_even_if_a_round_holds_it() {
        // A round participation that never finishes must not let a coin
        // expire: inside the last quarter of the lead time it is asked for
        // again, at most hourly.
        let soon = now_unix() + 86_400; // lead is 7 days, so this is critical
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("c1", 5_000, soon)])));
        let rounds = Arc::new(std::sync::Mutex::new((
            200,
            r#"[{"id": 9, "state": "pending", "participation": {"inputs": ["c1"], "outputs": []}}]"#
                .to_string(),
        )));
        let (engine, db, rec) = renewal_engine(vtxos, rounds, (200, "{}"));
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        // Asked within the last hour: not again on the next cycles.
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        assert_eq!(refresh_calls(&rec).len(), 1);
        // An hour later it is asked for once more.
        db.call(|c| {
            let mut list = load_requested(c)?;
            for r in &mut list {
                r.at -= LAST_RESORT_RETRY_SECS + 1;
            }
            save_requested(c, &list)
        })
        .await
        .unwrap();
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        assert_eq!(refresh_calls(&rec).len(), 2);
    }

    #[tokio::test]
    async fn a_large_coin_is_not_renewed_at_a_fee_to_rescue_a_tiny_one() {
        // On-chain payout: the server may hold one large coin. Renewing it
        // outside the free window costs far more than the 136 sats rescued.
        let soon = now_unix() + 86_400;
        let later = now_unix() + 20 * 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[
            ("tiny", 136, soon),
            ("big", 900_000, later),
        ])));
        let rounds = Arc::new(std::sync::Mutex::new((200, "[]".to_string())));
        let (engine, db, rec) = renewal_engine(vtxos, rounds, (200, "{}"));
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 0);
        assert!(refresh_calls(&rec).is_empty());
        assert!(renew_rows(&db).await.is_empty());
        let sats = db
            .call(|c| crate::settings::get(c, TOO_SMALL_SATS_KEY))
            .await
            .unwrap();
        assert_eq!(sats.as_deref(), Some("136"));
    }

    #[tokio::test]
    async fn during_an_emergency_withdrawal_other_coins_are_still_renewed_but_nothing_is_sent() {
        // A payment that arrives while a withdrawal runs must not expire.
        let soon = now_unix() + 86_400;
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/exits/status/live" => (
                200,
                r#"[{"vtxo_id": "exiting", "state": {"type": "awaiting-delta"}}]"#.to_string(),
            ),
            "/api/v1/wallet/vtxos" => (
                200,
                coins(&[("exiting", 150_000, soon), ("fresh", 5_000, soon)]),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 155000}"#.to_string()),
            "/api/v1/wallet/rounds" => (200, "[]".to_string()),
            "/api/v1/wallet/refresh/vtxos" => (200, "{}".to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let report = engine.sweep_now().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 1);
        assert!(report.swept_sats.is_none());
        assert!(report
            .sweep_refused
            .unwrap()
            .contains("emergency withdrawal"));
        let calls = refresh_calls(&recorded);
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].contains("fresh") && !calls[0].contains("exiting"),
            "{}",
            calls[0]
        );
        assert!(!recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
    }

    #[tokio::test]
    async fn an_already_in_a_round_refusal_is_recorded_once_and_is_not_a_failure() {
        let soon = now_unix() + 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("c1", 5_000, soon)])));
        // Round list unreadable, and the server says a round attempt holds
        // the inputs.
        let rounds = Arc::new(std::sync::Mutex::new((404, "{}".to_string())));
        let (engine, db, _rec) = renewal_engine(
            vtxos,
            rounds,
            (
                400,
                r#"{"message": "inputs are already held by a round attempt"}"#,
            ),
        );
        for _ in 0..3 {
            engine.tick().await.unwrap();
        }
        assert_eq!(renew_rows(&db).await, vec![("waiting".to_string(), 5_000)]);
    }

    #[tokio::test]
    async fn a_failed_round_counts_as_a_failed_renewal_and_is_retried() {
        let soon = now_unix() + 86_400;
        let vtxos = Arc::new(std::sync::Mutex::new(coins(&[("c1", 5_000, soon)])));
        let rounds = Arc::new(std::sync::Mutex::new((200, "[]".to_string())));
        let (engine, db, rec) = renewal_engine(vtxos, rounds.clone(), (200, "{}"));
        engine.tick().await.unwrap();
        *rounds.lock().unwrap() = (
            200,
            r#"[{"id": 3, "state": "failed", "participation": {"inputs": ["c1"], "outputs": []}}]"#
                .to_string(),
        );
        // The failed round is noticed; the coin is free again and asked for
        // once more in the same cycle.
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        assert_eq!(refresh_calls(&rec).len(), 2);
        assert_eq!(
            renew_rows(&db).await,
            vec![
                ("failed".to_string(), 5_000),
                ("requested".to_string(), 5_000)
            ]
        );
    }

    #[tokio::test]
    async fn an_emergency_withdrawal_pauses_renewals_and_sweeps() {
        let soon = now_unix() + 86_400;
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/exits/status/live" => (
                200,
                r#"[{"vtxo_id": "v1", "state": {"type": "processing"}}]"#.to_string(),
            ),
            "/api/v1/wallet/vtxos" => (
                200,
                format!(
                    r#"[{{"id": "v1", "amount_sat": 150000, "expires_at": {soon}, "state": {{"type": "spendable"}}}}]"#
                ),
            ),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 150000}"#.to_string()),
            _ => (200, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let engine = test_engine(
            client,
            db.clone(),
            Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.refreshed_vtxos, 0);
        assert!(report.swept_sats.is_none());
        let report = engine.sweep_now().await.unwrap();
        assert!(report.swept_sats.is_none());
        assert!(report
            .sweep_refused
            .unwrap()
            .contains("emergency withdrawal"));
        assert!(!recorded.lock().unwrap().iter().any(|r| {
            r.route() == "/api/v1/wallet/refresh/vtxos" || r.route() == "/api/v1/wallet/send"
        }));
    }

    #[tokio::test]
    async fn a_started_withdrawal_stays_paused_when_barkd_cannot_say_and_flags_claimable() {
        let state = Arc::new(std::sync::Mutex::new((404u16, "{}".to_string())));
        let s2 = state.clone();
        let (base, recorded) = mock::spawn(move |req| match req.route() {
            "/api/v1/exits/status/live" => s2.lock().unwrap().clone(),
            "/api/v1/wallet/vtxos" => (200, "[]".to_string()),
            "/api/v1/wallet/balance" => (200, r#"{"spendable_sat": 0}"#.to_string()),
            _ => (404, "{}".to_string()),
        });
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(&base, "t"),
        )
        .unwrap();
        let db = Db::open(":memory:").unwrap();
        let started = (now_unix() - 600).to_string();
        db.call(move |c| {
            crate::settings::set(c, crate::emergency_exit::EXIT_STARTED_AT_KEY, &started)
        })
        .await
        .unwrap();
        let engine = test_engine(client, db.clone(), None);
        engine.tick().await.unwrap();
        assert!(
            !recorded
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.route() == "/api/v1/wallet/vtxos"),
            "paused before reading coins"
        );
        // Ready to claim: the flag Home and the phone alert read.
        *state.lock().unwrap() = (
            200,
            r#"[{"vtxo_id": "v1", "state": {"type": "claimable"}}]"#.to_string(),
        );
        engine.tick().await.unwrap();
        let flag = db
            .call(|c| crate::settings::get(c, crate::emergency_exit::EXIT_CLAIMABLE_KEY))
            .await
            .unwrap();
        assert_eq!(flag.as_deref(), Some("true"));
        // Claimed and gone: the pause ends by itself.
        *state.lock().unwrap() = (200, "[]".to_string());
        engine.tick().await.unwrap();
        let (started, flag) = db
            .call(|c| {
                Ok((
                    crate::settings::get(c, crate::emergency_exit::EXIT_STARTED_AT_KEY)?,
                    crate::settings::get(c, crate::emergency_exit::EXIT_CLAIMABLE_KEY)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!((started, flag), (None, None));
        assert!(
            recorded
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.route() == "/api/v1/wallet/vtxos"),
            "running normally again"
        );
    }

    #[test]
    fn expiry_conversion_handles_heights_and_unknowns() {
        use crate::wallet::barkd::{Tip, Vtxo};
        let tip = Tip {
            height: 800,
            timestamp: 1_000_000,
        };
        let mk = |h: Option<u64>, t: Option<i64>| Vtxo {
            id: "v".into(),
            amount_sats: 1,
            expiry_height: h,
            expires_at: t,
        };
        assert_eq!(vtxo_expiry_unix(&mk(None, Some(5)), Some(&tip)), Some(5));
        assert_eq!(
            vtxo_expiry_unix(&mk(Some(900), None), Some(&tip)),
            Some(1_060_000)
        );
        assert_eq!(vtxo_expiry_unix(&mk(Some(700), None), Some(&tip)), Some(0));
        assert_eq!(vtxo_expiry_unix(&mk(Some(900), None), None), None);
        assert_eq!(vtxo_expiry_unix(&mk(None, None), Some(&tip)), None);
    }
}
