//! Settlement engine (Phase 1, Custody Level 0).
//!
//! Two jobs, in priority order:
//!
//! 1. **Renewal** — refresh spendable VTXOs inside the renewal window,
//!    before their per-VTXO expiry (learn_lifetime.md). Renewal never moves
//!    funds.
//! 2. **Sweep** — move the excess above the till cap (`hot_wallet_cap_sats`
//!    for an Ark payout address, `onchain_sweep_cap_sats` for an on-chain one) to the
//!    merchant's own wallet: an Ark send when the destination is on the
//!    same ASP (e.g. their Arké wallet), or a cooperative offboard of the
//!    whole till when it is an on-chain address (learn_offboard.md).
//!    Emergency exits are never automated: the owner starts one from
//!    Settings (`routes::exit`), and while it runs this engine pauses.
//!
//! An automatic ON-CHAIN sweep also waits until the network fee rate is at or
//! below `onchain_sweep_max_fee_sat_vb`; the merchant can always sweep by hand.

use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection};

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
}

/// Bitcoin's target block time, used to turn blocks into seconds.
const SECS_PER_BLOCK: i64 = 600;

/// Never wait until fewer blocks than this are left (about two hours): room
/// for a refresh to be retried.
/// An automatic sweep only goes ahead when at least this many sats are above
/// the amount the server keeps. Below it, the money stays and the next payment
/// adds to it.
pub const MIN_SWEEP_EXCESS_SATS: i64 = 1_000;

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
/// A requested renewal that cannot be confirmed either way after this long
/// is counted as failed (and its coins become eligible again).
const REQUESTED_GIVE_UP_SECS: i64 = 2 * 3600;
/// Give barkd a moment to list a fresh participation before judging it.
const REQUESTED_GRACE_SECS: i64 = 60;

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
    let age = now - r.at;
    if rounds_known && age >= REQUESTED_GRACE_SECS {
        return RenewalOutcome::Failed(
            "the coins were not taken into a round; they will be renewed again",
        );
    }
    if age >= REQUESTED_GIVE_UP_SECS {
        return RenewalOutcome::Failed(
            "no sign of the round finishing; the coins will be renewed again",
        );
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
    /// WHOLE till to the payout address, ignoring the till cap and the
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
    ) -> Result<std::collections::HashSet<String>, AppError> {
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
        let waiting: HashSet<String> = self
            .db
            .call(move |conn| {
                let list = load_requested(conn)?;
                if list.is_empty() {
                    return Ok(HashSet::new());
                }
                let p: HashSet<&str> = present_c.iter().map(String::as_str).collect();
                let h: HashSet<&str> = held_c.iter().map(String::as_str).collect();
                let d: HashSet<&str> = dead_c.iter().map(String::as_str).collect();
                let mut keep = Vec::new();
                let mut waiting = HashSet::new();
                for r in list {
                    match judge_requested(&r, &p, &h, &d, rounds_known, now) {
                        RenewalOutcome::Waiting => {
                            waiting.extend(r.inputs.iter().cloned());
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
                Ok(waiting)
            })
            .await?;
        held.extend(waiting);
        Ok(held)
    }

    /// While an emergency withdrawal runs, renewals and sweeps would fight
    /// it over the same coins, so the whole cycle pauses. Also keeps the
    /// "ready to claim" flag (Home, phone alert) up to date, and notices when
    /// the withdrawal is over. `Some(reason)` = paused.
    async fn exit_pause(&self) -> Result<Option<String>, AppError> {
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
                }
                Ok(())
            })
            .await?;
        let running = live.as_ref().is_some_and(|l| !l.is_empty());
        let paused = (started.is_some() && !over) || running;
        Ok(paused.then(|| {
            "An emergency withdrawal is in progress, so normal transfers and renewals are paused.".to_string()
        }))
    }

    async fn cycle(&self, manual: bool) -> Result<TickReport, AppError> {
        let _one_at_a_time = CYCLE_LOCK.lock().await;
        let mut report = TickReport::default();
        if let Some(reason) = self.exit_pause().await? {
            if manual {
                report.sweep_refused = Some(reason);
            }
            return Ok(report);
        }

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
        let excluded = self
            .settle_requested_renewals(&vtxos, participations.as_deref(), now)
            .await?;

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
            extra.sort_by_key(|v| vtxo_expiry_unix(v, tip.as_ref()).unwrap_or(i64::MAX));
            let mut total = due_total;
            for v in extra {
                if total >= MIN_RENEW_SATS {
                    break;
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
        self.db
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
            .await?;

        let due_ids: Vec<String> = due
            .iter()
            .map(|v| v.id.clone())
            .filter(|id| !id.is_empty())
            .collect();
        let mut renewal_accepted = false;
        if !due_ids.is_empty() {
            match self.client.refresh_vtxos(&due_ids).await {
                Ok(_) => {
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
                                    "asked to renew {count} coins in the Ark server's next round"
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

        // 2. Cap-driven sweep: move the excess above the hot cap to the
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

        let offchain = balances.offchain_sats;
        // Ark payout: low cap (almost free). On-chain payout: higher cap
        // (every withdrawal pays a network fee).
        let cap = if manual {
            0
        } else {
            let config = self.config.clone();
            let dest = destination.clone();
            self.db
                .call(move |conn| crate::payout::cap_for(conn, &config, &dest))
                .await?
        };
        let excess = offchain.saturating_sub(cap);
        if excess <= 0 {
            return Ok(report);
        }
        // An automatic sweep waits until the excess is worth sending, so the
        // payout wallet receives a few bigger coins instead of many tiny ones.
        // "Move my money now" always sends whatever is there.
        if !manual && excess < MIN_SWEEP_EXCESS_SATS {
            return Ok(report);
        }

        // Last line of defence: re-check the address at the moment of use (the
        // same checks as when it was saved: checksum, form, network). Anything
        // that fails is refused and shown on the dashboard, never sent.
        if let Err(why) = crate::payout::check(&destination, &self.config.mode) {
            let reason = format!("payout address refused: {why}");
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
        *self.last_refusal.lock().unwrap_or_else(|e| e.into_inner()) = None;

        let (amount, result) = if ark {
            (excess, self.client.ark_send(&destination, excess).await)
        } else {
            // R8: offboard only enough whole VTXOs to cover the excess
            // (largest first) via /wallet/offboard/vtxos, leaving the rest
            // as the till. Overshoot is at most one VTXO.
            let (ids, total) = select_vtxos_covering(&vtxos.spendable, excess);
            if ids.is_empty() {
                return Ok(report);
            }
            (total, self.client.offboard_vtxos(&destination, &ids).await)
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
    record_id(conn, kind, amount_sats, destination, status, detail).map(|_| ())
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
    conn.execute(
        "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            kind,
            amount_sats,
            destination,
            status,
            detail,
            now_unix()
        ],
    )?;
    Ok(id)
}

fn delete_setting(conn: &Connection, key: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}

fn load_requested(conn: &Connection) -> Result<Vec<RequestedRenewal>, AppError> {
    Ok(crate::settings::get(conn, REQUESTED_RENEWALS_KEY)?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
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

    fn test_config(destination: Option<&str>, cap: i64) -> Arc<Config> {
        Arc::new(Config {
            settlement_destination: destination.map(str::to_string),
            hot_wallet_cap_sats: cap,
            onchain_sweep_cap_sats: cap,
            renewal_lead_time_secs: 604_800,
            ..Config::default()
        })
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
        let engine = SettlementEngine::new(client, db.clone(), test_config(None, 100_000));
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(50_000));
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
    async fn sweeps_excess_over_the_cap_via_ark_send() {
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(
                Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
                100_000,
            ),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(50_000));

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
        assert_eq!(amount_sats, 50_000);

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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"), 100_000),
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(Some("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"), 100_000),
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

    fn engine_for(base: &str, db: &Db, destination: &str, cap: i64) -> SettlementEngine {
        let client = crate::wallet::barkd::BarkdClient::new(
            crate::wallet::barkd::BarkdConfig::new(base, "t"),
        )
        .unwrap();
        SettlementEngine::new(client, db.clone(), test_config(Some(destination), cap))
    }

    #[tokio::test]
    async fn an_automatic_sweep_waits_for_a_minimum_excess_but_the_button_does_not() {
        const ARK: &str =
            "tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh";
        // The server keeps 1,000. With 1,999 held the excess is 999: not sent.
        // With 2,000 held the excess is 1,000: sent. "Move my money now" always sends.
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
            (engine_for(&base, &db, ARK, 1_000), base)
        };

        let (engine, _base) = engine_holding(1_999).await;
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        assert_eq!(engine.sweep_now().await.unwrap().swept_sats, Some(1_999));

        let (engine, _base) = engine_holding(2_000).await;
        assert_eq!(engine.tick().await.unwrap().swept_sats, Some(1_000));
    }

    #[tokio::test]
    async fn automatic_onchain_sweep_waits_for_low_fees() {
        // No on-chain reserve at all, fees 80 > the 50 limit: nothing is sent,
        // one calm "waiting" row is written (not one per tick).
        let (base, recorded) = fee_mock(80, 150_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(
            &base,
            &db,
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            100_000,
        );
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
    async fn automatic_onchain_sweep_goes_when_fees_are_low_even_without_a_reserve() {
        let (base, recorded) = fee_mock(50, 150_000); // exactly at the limit
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(
            &base,
            &db,
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            100_000,
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/offboard/vtxos"));
    }

    #[tokio::test]
    async fn manual_sweep_ignores_the_fee_limit_and_the_cap() {
        // Fees 200 and the till (5,000) is far under the 100,000 cap: the
        // automatic path does nothing, the button moves everything.
        let (base, recorded) = fee_mock(200, 5_000);
        let db = Db::open(":memory:").unwrap();
        let engine = engine_for(
            &base,
            &db,
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            100_000,
        );
        assert!(engine.tick().await.unwrap().swept_sats.is_none());
        let report = engine.sweep_now().await.unwrap();
        assert_eq!(report.swept_sats, Some(5_000));
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
        let engine = engine_for(
            &base,
            &db,
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            100_000,
        );
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(
                Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
                100_000,
            ),
        );
        let report = engine.tick().await.unwrap();
        assert_eq!(report.swept_sats, Some(100_001));
        assert!(report.sweep_refused.is_none());
        let sent = recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send");
        assert!(sent, "the Ark send must be made");
    }

    #[test]
    fn each_payout_type_has_its_own_cap() {
        let c = Config {
            hot_wallet_cap_sats: 20_000,
            onchain_sweep_cap_sats: 1_000_000,
            ..Config::default()
        };
        assert_eq!(c.sweep_cap_for("ark1abc"), 20_000);
        assert_eq!(c.sweep_cap_for("TARK1abc"), 20_000);
        assert_eq!(c.sweep_cap_for("bc1qabc"), 1_000_000);
        assert_eq!(c.sweep_cap_for("tb1qabc"), 1_000_000);
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(
                Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
                100_000,
            ),
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
        let engine = SettlementEngine::new(client, db, test_config(None, 100_000));
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
        let engine = SettlementEngine::new(
            client,
            Db::open(":memory:").unwrap(),
            test_config(None, 100_000),
        );
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
        SettlementEngine::new(
            client,
            Db::open(":memory:").unwrap(),
            test_config(None, 100_000),
        )
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
    /// balance well above the cap, and a refresh answer chosen by the test.
    fn due_and_rich_engine(
        refresh_status: u16,
    ) -> (
        SettlementEngine,
        Db,
        std::sync::Arc<std::sync::Mutex<Vec<mock::Recorded>>>,
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(
                Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
                100_000,
            ),
        );
        (engine, db, recorded)
    }

    #[tokio::test]
    async fn a_failed_renewal_is_recorded_so_home_can_warn() {
        let (engine, db, recorded) = due_and_rich_engine(500);
        let report = engine.tick().await.unwrap();
        // A failing renewal must never hold withdrawals back: that is when
        // the owner most needs the money to keep moving out.
        assert_eq!(report.swept_sats, Some(50_000));
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/send"));
        // ... and "move my money now" works too (whole till, no cap).
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
        // The owner's button reads the wallet again after the renewal and
        // moves what is spendable (the mock still reports the full balance).
        let report = engine.sweep_now().await.unwrap();
        assert_eq!(report.swept_sats, Some(150_000));
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
        let engine = SettlementEngine::new(
            client,
            Db::open(":memory:").unwrap(),
            test_config(None, 100_000),
        );
        assert_eq!(engine.tick().await.unwrap().refreshed_vtxos, 1);
        assert!(recorded
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.route() == "/api/v1/wallet/refresh/vtxos"));
    }

    // -- Renewal on mainnet (MAINNET_PLAN PR D) ------------------------------

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
        // Coins back, no round holds them: failed (after the grace period).
        assert!(matches!(
            judge_requested(&r, &set(&["b"]), &none, &none, true, now),
            RenewalOutcome::Failed(_)
        ));
        let fresh = RequestedRenewal {
            at: now - 5,
            ..r.clone()
        };
        assert_eq!(
            judge_requested(&fresh, &set(&["b"]), &none, &none, true, now),
            RenewalOutcome::Waiting
        );
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
        Arc<std::sync::Mutex<Vec<mock::Recorded>>>,
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
        let engine = SettlementEngine::new(client, db.clone(), test_config(None, 100_000));
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

    fn refresh_calls(rec: &Arc<std::sync::Mutex<Vec<mock::Recorded>>>) -> Vec<String> {
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
        let soon = now_unix() + 86_400;
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
        let engine = SettlementEngine::new(
            client,
            db.clone(),
            test_config(
                Some("tark1pe2tczy3pqflsjdvj46amgam8k4j4dxwd4wxgd39jfawf8caj9g7z2wy9d6dlzqq209rjh"),
                1_000,
            ),
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
        let engine = SettlementEngine::new(client, db.clone(), test_config(None, 1_000));
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
