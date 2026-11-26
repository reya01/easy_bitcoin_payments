//! Settings → "Emergency: withdraw without the Ark server" (archived mainnet
//! plan, PR G). Every action needs a signed-in session, the admin password AND a
//! two-step code (two-step sign-in must be on), and is audit-logged. Nothing
//! here runs on a timer: an exit only ever starts from the owner's button.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::emergency_exit as ee;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;
use crate::wallet::barkd::{vtxo_expiry_unix, BarkdClient};

/// Settings key: the claim address chosen when the exit was started.
use crate::emergency_exit::EXIT_DESTINATION_KEY;

fn client(state: &AppState) -> Result<BarkdClient, AppError> {
    BarkdClient::from_app_config(&state.config).map_err(AppError::from)
}

/// The real-money lock, judged by the mode and by the wallet's own network.
/// With `strict` (before anything moves money) an unreadable network also
/// refuses: the lock must not open because a request failed.
async fn switched_off(state: &AppState, c: &BarkdClient, strict: bool) -> Option<&'static str> {
    let net = c.ark_network().await.ok().flatten();
    if strict && net.is_none() {
        return Some(
            "Could not confirm which network the wallet is on right now; try again in a minute.",
        );
    }
    ee::not_enabled_reason(&state.config.mode, net.as_deref())
}

fn audit(conn: &rusqlite::Connection, action: &str, detail: &str) {
    if let Ok(Some(store)) = crate::store::first(conn) {
        let _ = crate::audit::record(conn, &store.id, action, Some(detail));
    }
}

fn short(addr: &str) -> String {
    if addr.len() <= 16 {
        return addr.to_string();
    }
    format!("{}…{}", &addr[..8], &addr[addr.len() - 6..])
}

/// The payout address, if it is an on-chain one (a natural claim address).
async fn onchain_payout(state: &AppState) -> Option<String> {
    let cfg = state.config.settlement_destination.clone();
    let mode = state.config.mode.clone();
    state
        .db
        .call(move |c| crate::payout::effective(c, cfg.as_deref()))
        .await
        .ok()
        .flatten()
        .filter(|a| ee::check_destination(a, &mode).is_ok())
}

/// Signed in, two-step sign-in on, the admin password right, and the code.
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    password: String,
    code: Option<String>,
) -> Result<(), AppError> {
    require_session(state, headers).await?;
    if !state.db.call(|c| crate::totp::is_enabled(c)).await? {
        return Err(AppError::BadRequest(
            "Turn on two-step sign-in in Settings first: an emergency withdrawal needs your admin password and a code from your authenticator app.".into(),
        ));
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    if !aa::verify_login_slow(stored, password).await? {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(state, headers, code).await
}

/// `POST /admin/exit/status` — everything the page shows before (and while)
/// an emergency withdrawal runs. Read-only; every wallet figure is best
/// effort, because this page matters most when things are broken.
pub async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let c = client(&state)?;
    let (two_step_on, started_at, destination, opted_in) = state
        .db
        .call(|conn| {
            Ok((
                crate::totp::is_enabled(conn)?,
                crate::settings::get(conn, ee::EXIT_STARTED_AT_KEY)?
                    .and_then(|v| v.parse::<i64>().ok()),
                crate::settings::get(conn, EXIT_DESTINATION_KEY)?,
                ee::opted_in(conn)?,
            ))
        })
        .await?;
    let payout = onchain_payout(&state).await;
    let claim_to = destination.clone().or_else(|| payout.clone());

    // A deposit only shows once the wallet has looked at the chain again.
    let _ = c.onchain_sync().await;
    let ark_sats = c.balances().await.ok().map(|b| b.offchain_sats);
    let onchain = c.onchain_balance_raw().await.ok();
    let confirmed = onchain.as_ref().and_then(ee::confirmed_onchain_sats);
    let pending = onchain.as_ref().map(ee::pending_onchain_sats).unwrap_or(0);
    let fee_answer = c.exit_fee(claim_to.as_deref()).await;
    let fee_error = fee_answer.as_ref().err().map(|e| e.to_string());
    let fee = fee_answer.ok().as_ref().and_then(ee::parse_fee);
    let unpriceable = fee.is_none() && fee_error.as_deref().is_some_and(ee::fee_cannot_be_priced);
    let preparing_since = state
        .db
        .call(|conn| crate::settings::get(conn, ee::EXIT_PREPARE_AT_KEY))
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|t| crate::util::now_unix() - t < ee::PREPARE_SHOWN_SECS);
    let connected = c.connected().await.ok().and_then(|x| x.connected);
    let live = c
        .exits_live()
        .await
        .ok()
        .map(|v| ee::parse_live(&v))
        .unwrap_or_default();
    let deadline = {
        let vtxos = c.vtxos().await.ok();
        let tip = c.bitcoin_tip().await.ok();
        let delta = c.exit_delta_blocks().await.ok().flatten();
        let expiries: Vec<i64> = vtxos
            .iter()
            .flat_map(|s| s.spendable.iter().chain(s.pending.iter()))
            .filter_map(|v| vtxo_expiry_unix(v, tip.as_ref()))
            .collect();
        ee::start_deadline(&expiries, delta)
    };
    let off = switched_off(&state, &c, false).await;
    let reserve_target = ee::reserve_target(fee.as_ref());
    let refusal = match (off, ark_sats) {
        (Some(off), _) => Some(off.to_string()),
        (None, Some(a)) => ee::refusal_explained(a, fee.as_ref(), fee_error.as_deref()),
        (None, None) => {
            Some("Could not read the balance on the server right now; try again later.".into())
        }
    };
    let live_view: Vec<Value> = live
        .iter()
        .map(|e| json!({ "vtxo_id": e.vtxo_id, "state": e.state, "words": ee::state_words(&e.state) }))
        .collect();
    Ok(Json(json!({
        "min_exit_sats": ee::MIN_EXIT_SATS,
        "ark_sats": ark_sats,
        "fee": fee,
        "onchain_confirmed_sats": confirmed,
        "onchain_pending_sats": pending,
        "fundable": fee.as_ref().and_then(|f| f.fundable),
        "deposit_needed_sats": fee.map(|f| ee::deposit_still_needed(&f, confirmed.unwrap_or(0))),
        "deadline": deadline,
        "asp_connected": connected,
        "two_step_on": two_step_on,
        "allowed": refusal.is_none(),
        "refusal": refusal,
        "unpriceable": unpriceable,
        "switched_off": off.is_some(),
        "opted_in": opted_in,
        "reserve_floor_sats": ee::RESERVE_FLOOR_SATS,
        "reserve_short_sats": confirmed.map(ee::reserve_short_sats),
        "reserve_min_sats": ee::RESERVE_MIN_SATS,
        "reserve_target_sats": reserve_target,
        "reserve_high_sats": ee::reserve_high(reserve_target),
        "preparing_since": preparing_since,
        "in_progress_since": started_at,
        "live": live_view,
        "claimable": ee::any_claimable(&live),
        "destination": claim_to,
    })))
}

/// `POST /admin/exit/prepare` — ask the wallet to refresh its coins so an
/// emergency withdrawal can be priced. The page calls this by itself when
/// the fee cannot be worked out because of coins received over Lightning
/// (drill: a refresh turns them into standard coins and the price appears).
/// Not an exit: no coin leaves the wallet, and a refresh costs at most a few
/// hundred sats. Behind the same real-money lock as the exit itself, only
/// when the problem is really there, and at most once an hour.
pub async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let c = client(&state)?;
    if let Some(off) = switched_off(&state, &c, true).await {
        return Err(AppError::BadRequest(off.to_string()));
    }
    let running = state
        .db
        .call(|conn| crate::settings::get(conn, ee::EXIT_STARTED_AT_KEY))
        .await?
        .is_some();
    if running {
        return Ok(Json(
            json!({ "started": false, "why": "an emergency withdrawal is already running" }),
        ));
    }
    let answer = c.exit_fee(None).await;
    let unpriceable = answer
        .as_ref()
        .err()
        .is_some_and(|e| ee::fee_cannot_be_priced(&e.to_string()));
    if !unpriceable {
        return Ok(Json(json!({ "started": false, "why": "not needed" })));
    }
    let now = crate::util::now_unix();
    let last = state
        .db
        .call(|conn| crate::settings::get(conn, ee::EXIT_PREPARE_AT_KEY))
        .await?
        .and_then(|v| v.parse::<i64>().ok());
    if let Some(wait) = ee::prepare_wait_secs(last, now) {
        return Ok(Json(
            json!({ "started": false, "why": "already asked recently", "retry_in_secs": wait }),
        ));
    }
    // Record first: a failed attempt must not be retried every minute.
    state
        .db
        .call(move |conn| {
            crate::settings::set(conn, ee::EXIT_PREPARE_AT_KEY, &now.to_string())?;
            audit(
                conn,
                "exit.prepare",
                "refresh asked for so the exit fee can be worked out",
            );
            Ok(())
        })
        .await?;
    c.refresh_all().await.map_err(AppError::from)?;
    Ok(Json(json!({ "started": true })))
}

/// `POST /admin/exit/deposit-address` — the server's own on-chain address,
/// where the owner sends the money that pays the exit fees.
pub async fn deposit_address(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let address = client(&state)?.onchain_address().await?;
    // Remembered, so money that arrives here is shown as fee money under
    // Alerts instead of money from an unknown place.
    let remembered = address.clone();
    state
        .db
        .call(move |conn| crate::invoice::remember_own_address(conn, &remembered, "exit_fees"))
        .await?;
    Ok(Json(json!({ "address": address })))
}

#[derive(Deserialize)]
pub struct ExitRequest {
    pub admin_password: String,
    #[serde(default)]
    pub code: Option<String>,
    /// Claim address (on-chain). For start, claim and the leftover button.
    #[serde(default)]
    pub destination: Option<String>,
    /// The tick-box: "I understand this is for emergencies ...".
    #[serde(default)]
    pub understood: bool,
}

/// Where a claim or the left-over money goes: the address chosen when the
/// withdrawal was started always wins, so a later request cannot redirect
/// it. Only when none was saved (a withdrawal this server did not start, or
/// fee money deposited for one that never started) is the typed address
/// used, and the owner's phone is told.
fn settled_destination(
    req: &ExitRequest,
    saved: Option<String>,
    mode: &str,
) -> Result<(String, bool), AppError> {
    match saved.filter(|s| !s.trim().is_empty()) {
        Some(saved) => ee::check_destination(&saved, mode)
            .map(|d| (d, false))
            .map_err(AppError::BadRequest),
        None => destination_from(req, None, mode).map(|d| (d, true)),
    }
}

/// Where the left-over fee money goes: the address saved at the start of a
/// withdrawal that is still on record, otherwise the address typed now.
fn leftover_destination(
    req: &ExitRequest,
    saved: Option<String>,
    started: bool,
    mode: &str,
) -> Result<String, AppError> {
    settled_destination(req, saved.filter(|_| started), mode).map(|(d, _)| d)
}

fn destination_from(
    req: &ExitRequest,
    fallback: Option<String>,
    mode: &str,
) -> Result<String, AppError> {
    let raw = req
        .destination
        .clone()
        .filter(|d| !d.trim().is_empty())
        .or(fallback)
        .ok_or_else(|| {
            AppError::BadRequest(
                "Enter the Bitcoin address (on-chain) the money should go to.".into(),
            )
        })?;
    ee::check_destination(&raw, mode).map_err(AppError::BadRequest)
}

/// `POST /admin/exit/start` — start the emergency withdrawal of every coin.
/// Refused unless it is worth it (fresh fee estimate, checked here, not only
/// in the page) and the confirmed on-chain balance already covers the fees.
pub async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ExitRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Fast refusal by mode; the wallet's own network is checked again after
    // sign-in, before the wallet is asked to move anything.
    if let Some(off) = ee::not_enabled_reason(&state.config.mode, None) {
        return Err(AppError::BadRequest(off.into()));
    }
    if !req.understood {
        return Err(AppError::BadRequest(
            "Tick the box to confirm you understand this is for emergencies, costs network fees and takes at least a day.".into(),
        ));
    }
    let payout = onchain_payout(&state).await;
    let destination = destination_from(&req, payout, &state.config.mode)?;
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    if let Some(off) = switched_off(&state, &c, true).await {
        return Err(AppError::BadRequest(off.into()));
    }
    let already = state
        .db
        .call(|c| crate::settings::get(c, ee::EXIT_STARTED_AT_KEY))
        .await?;
    if already.is_some() {
        return Err(AppError::BadRequest(
            "An emergency withdrawal is already in progress.".into(),
        ));
    }

    let ark_sats = c.balances().await?.offchain_sats;
    let fee_answer = c.exit_fee(Some(&destination)).await;
    let fee_error = fee_answer.as_ref().err().map(|e| e.to_string());
    let fee = fee_answer.ok().as_ref().and_then(ee::parse_fee);
    if let Some(why) = ee::refusal_explained(ark_sats, fee.as_ref(), fee_error.as_deref()) {
        return Err(AppError::BadRequest(why));
    }
    let Some(fee) = fee else {
        return Err(AppError::BadRequest(
            "Could not estimate the fees right now; try again later.".into(),
        ));
    };
    let _ = c.onchain_sync().await;
    let confirmed = c
        .onchain_balance_raw()
        .await
        .ok()
        .as_ref()
        .and_then(ee::confirmed_onchain_sats)
        .unwrap_or(0);
    let need = ee::deposit_still_needed(&fee, confirmed);
    if need > 0 {
        return Err(AppError::BadRequest(format!(
            "The network fees are paid from the server's own on-chain balance, which needs {need} more confirmed sats. Send them to the deposit address shown on this page, wait for 1 confirmation, then try again."
        )));
    }

    if let Some(why) = ee::start_blocker(&fee) {
        return Err(AppError::BadRequest(why));
    }

    // Save where the money goes BEFORE asking the wallet: if the start goes
    // through but its answer is lost, claiming still uses this address.
    let dest_saved = destination.clone();
    state
        .db
        .call(move |conn| crate::settings::set(conn, EXIT_DESTINATION_KEY, &dest_saved))
        .await?;
    // No automatic transfer may run while the coins are handed to the withdrawal.
    let _cycle = crate::wallet::settlement::lock_cycle().await;
    c.start_exit_all().await?;
    let now = crate::util::now_unix();
    let detail = format!(
        "emergency withdrawal started: {ark_sats} sats, about {} sats in network fees, to {}",
        fee.total_sat,
        short(&destination)
    );
    state
        .db
        .call(move |conn| {
            crate::settings::set(conn, ee::EXIT_STARTED_AT_KEY, &now.to_string())?;
            crate::settings::set(conn, EXIT_DESTINATION_KEY, &destination)?;
            audit(conn, "exit.started", &detail);
            record_exit_movement(conn, "exit_start", ark_sats, &destination, &detail)?;
            let _ = crate::alerts::queue_notice(conn, "exit_started");
            Ok(())
        })
        .await?;
    Ok(Json(json!({
        "started": true,
        "message": "Started. The server now sends the exit transactions by itself over the next hours. Leave the server on, and download a fresh backup now: the backup carries the withdrawal, the 12 words do not.",
    })))
}

/// Write an emergency-withdrawal money movement to the transfers list
/// (`settlements`), which is kept for good: the security log is trimmed after
/// a year, and a year later the owner must still see where the coins went.
/// `amount_sats` is 0 when the wallet does not say (claiming, or sending
/// everything): the page then says to check the wallet. Found by review
/// (26 November 2026).
pub(crate) fn record_exit_movement(
    conn: &rusqlite::Connection,
    kind: &str,
    amount_sats: i64,
    destination: &str,
    detail: &str,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settlements (id, kind, amount_sats, destination, status, detail, created_at)
         VALUES (?1, ?2, ?3, ?4, 'broadcast', ?5, ?6)",
        rusqlite::params![
            crate::util::new_id("stl"),
            kind,
            amount_sats.max(0),
            destination,
            detail,
            crate::util::now_unix()
        ],
    )?;
    Ok(())
}

/// `POST /admin/exit/cancel` — stop the exits barkd still allows stopping.
pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ExitRequest>,
) -> Result<Json<Value>, AppError> {
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    let live = ee::parse_live(&c.exits_live().await?);
    let (mut stopped, mut refused) = (0usize, 0usize);
    for e in live
        .iter()
        .filter(|e| !matches!(e.state.as_str(), "claimable" | "claim-in-progress"))
    {
        match c.cancel_exit(&e.vtxo_id).await {
            Ok(_) => stopped += 1,
            Err(err) => {
                tracing::warn!(error = %err, "could not stop one exit");
                refused += 1;
            }
        }
    }
    let still = c.exits_live().await.ok().map(|v| ee::parse_live(&v).len());
    let over = still == Some(0);
    state
        .db
        .call(move |conn| {
            if over {
                conn.execute(
                    "DELETE FROM settings WHERE key IN (?1, ?2, ?3)",
                    rusqlite::params![
                        ee::EXIT_STARTED_AT_KEY,
                        ee::EXIT_CLAIMABLE_KEY,
                        EXIT_DESTINATION_KEY
                    ],
                )?;
            }
            audit(
                conn,
                "exit.stopped",
                &format!("emergency withdrawal stopped for {stopped} coin(s); {refused} could not be stopped"),
            );
            Ok(())
        })
        .await?;
    let message = if refused == 0 {
        format!("Stopped for {stopped} coin(s). Those coins are back to normal.")
    } else {
        format!("Stopped for {stopped} coin(s). {refused} could not be stopped any more (their last transaction is already out); they will finish and can be claimed.")
    };
    Ok(Json(
        json!({ "stopped": stopped, "not_stopped": refused, "message": message }),
    ))
}

/// `POST /admin/exit/claim` — the last step: send every claimable output to
/// the address. Never automatic.
pub async fn claim(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ExitRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let saved = state
        .db
        .call(|c| crate::settings::get(c, EXIT_DESTINATION_KEY))
        .await?;
    let (destination, typed_now) = settled_destination(&req, saved, &state.config.mode)?;
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    let live = ee::parse_live(&c.exits_live().await?);
    if !ee::any_claimable(&live) {
        return Err(AppError::BadRequest(
            "Nothing is ready to claim yet. The page says when it is.".into(),
        ));
    }
    let answer = c.claim_exits(&destination).await?;
    let txid = answer
        .get("txid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let detail = format!(
        "emergency withdrawal claimed to {}{}; the amount is what arrives after network fees",
        short(&destination),
        if txid.is_empty() {
            String::new()
        } else {
            format!(", transaction {txid}")
        }
    );
    state
        .db
        .call(move |conn| {
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                rusqlite::params![ee::EXIT_CLAIMABLE_KEY],
            )?;
            audit(conn, "exit.claimed", &detail);
            record_exit_movement(conn, "exit_claim", 0, &destination, &detail)?;
            let _ = crate::alerts::queue_notice(
                conn,
                if typed_now {
                    "exit_claimed_new_address"
                } else {
                    "exit_claimed"
                },
            );
            Ok(())
        })
        .await?;
    Ok(Json(json!({
        "claimed": true,
        "message": "Sent. It arrives at your address once the transaction confirms. If any fee money is left on the server, send it back with the button below.",
    })))
}

#[derive(Deserialize)]
pub struct OptInRequest {
    /// true = turn the emergency withdrawal on; false = turn it off.
    pub on: bool,
    pub admin_password: String,
    /// Authenticator code (or passkey approval), when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/exit/optin` — turn the emergency withdrawal on or off. It is
/// off until the owner turns it on. Turning it on needs what an emergency
/// withdrawal itself needs (two-step sign-in on, the admin password, a code).
/// While it is on the server keeps at least [`ee::RESERVE_FLOOR_SATS`] of
/// regular Bitcoin for the fees and never sends it out. Turning it off needs
/// the password and, when two-step sign-in is on, a code; it frees the fee
/// money to be taken back. Refused while a withdrawal runs: that
/// money is in use. Starting, stopping and claiming a withdrawal never depend
/// on this switch, so an emergency is never blocked by it.
pub async fn set_opt_in(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<OptInRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    if req.on {
        if let Some(off) = ee::not_enabled_reason(&state.config.mode, None) {
            return Err(AppError::BadRequest(off.into()));
        }
        authorize(&state, &headers, req.admin_password, req.code).await?;
    } else {
        let stored = state.db.call(|c| aa::stored_hash(c)).await?;
        if !aa::verify_login_slow(stored, req.admin_password).await? {
            return Err(AppError::BadRequest(
                "that admin password isn't right".into(),
            ));
        }
        crate::routes::admin_session::check_second_step(&state, &headers, req.code).await?;
        let running = state
            .db
            .call(|c| crate::settings::get(c, ee::EXIT_STARTED_AT_KEY))
            .await?
            .is_some();
        if running {
            return Err(AppError::BadRequest(
                "An emergency withdrawal is in progress. Wait until it has finished before turning this off: it is using the fee money.".into(),
            ));
        }
    }
    let on = req.on;
    state
        .db
        .call(move |conn| {
            ee::set_opted_in(conn, on)?;
            if on {
                audit(conn, "exit.turned_on", "emergency withdrawal turned on");
            } else {
                audit(conn, "exit.turned_off", "emergency withdrawal turned off");
            }
            Ok(())
        })
        .await?;
    let message = if on {
        "Turned on. The server now keeps at least 5,000 sats of regular Bitcoin for the fees and never sends that part out. Add it below."
    } else {
        "Turned off. Everything is sent on to your payout address as usual, and you can take the fee money back with the button below."
    };
    Ok(Json(json!({ "opted_in": on, "message": message })))
}

/// `POST /admin/exit/leftover` — send the fee money back. Refused while an exit
/// is still running (it may need that money). While the emergency withdrawal is
/// turned on, only what is above the 5,000-sat floor goes (after the fee).
pub async fn leftover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ExitRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Fast refusal by mode; the wallet's own network is checked again after
    // sign-in, before the wallet is asked to move anything.
    if let Some(off) = ee::not_enabled_reason(&state.config.mode, None) {
        return Err(AppError::BadRequest(off.into()));
    }
    // While the emergency withdrawal is turned on (and none has started), at
    // least 5,000 sats of this money stays as its fee reserve (see below).
    let (opted_in, started) = state
        .db
        .call(|c| {
            Ok((
                ee::opted_in(c)?,
                crate::settings::get(c, ee::EXIT_STARTED_AT_KEY)?.is_some(),
            ))
        })
        .await?;
    // The address saved when a withdrawal was started binds only while that
    // withdrawal is still on record. Otherwise the address typed now is used,
    // never a stale one from an earlier, stopped withdrawal.
    let saved = state
        .db
        .call(|c| crate::settings::get(c, EXIT_DESTINATION_KEY))
        .await?;
    let destination = leftover_destination(&req, saved, started, &state.config.mode)?;
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    if let Some(off) = switched_off(&state, &c, true).await {
        return Err(AppError::BadRequest(off.into()));
    }
    let live = ee::parse_live(&c.exits_live().await?);
    if live.iter().any(|e| e.state != "claim-in-progress") {
        return Err(AppError::BadRequest(
            "Wait until the emergency withdrawal has finished: it may still need this money for fees.".into(),
        ));
    }
    let plan = if opted_in && !started {
        let _ = c.onchain_sync().await;
        let confirmed = c
            .onchain_balance_raw()
            .await
            .ok()
            .as_ref()
            .and_then(ee::confirmed_onchain_sats)
            .unwrap_or(0);
        // The wallet sends at a rate aimed at the next few blocks: estimate with the fast rate.
        let rate = c.onchain_fee_rates().await.map_err(|_| {
            AppError::BadRequest(
                "Could not read the network fee right now, so nothing was sent. Try again in a few minutes.".into(),
            )
        })?;
        ee::leftover_plan(true, false, confirmed, rate.fast_sat_per_vb)
            .map_err(AppError::BadRequest)?
    } else {
        ee::Leftover::Everything
    };
    let answer = match plan {
        ee::Leftover::Everything => c.onchain_drain(&destination).await?,
        ee::Leftover::Amount(sats) => c.onchain_send(&destination, sats).await?,
    };
    let txid = answer
        .get("txid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tx = if txid.is_empty() {
        String::new()
    } else {
        format!(", transaction {txid}")
    };
    let detail = match plan {
        ee::Leftover::Everything => {
            format!("left-over on-chain money sent to {}{tx}", short(&destination))
        }
        ee::Leftover::Amount(sats) => format!(
            "{sats} sats of on-chain money sent to {}{tx} (5,000 kept for the emergency withdrawal)",
            short(&destination)
        ),
    };
    let amount = match plan {
        ee::Leftover::Everything => 0,
        ee::Leftover::Amount(sats) => sats,
    };
    state
        .db
        .call(move |conn| {
            audit(conn, "exit.leftover_sent", &detail);
            record_exit_movement(conn, "exit_leftover", amount, &destination, &detail)?;
            let _ = crate::alerts::queue_notice(conn, "exit_leftover_sent");
            Ok(())
        })
        .await?;
    let message = match plan {
        ee::Leftover::Everything => "Sent the left-over on-chain money to your address.".to_string(),
        ee::Leftover::Amount(sats) => format!(
            "Sent {} sats to your address. At least {} sats stays on the server for the emergency withdrawal's fees.",
            crate::payout::commas(sats),
            crate::payout::commas(ee::RESERVE_FLOOR_SATS)
        ),
    };
    Ok(Json(json!({
        "sent": true,
        "txid": txid,
        "message": message,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx";
    const TYPED: &str = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7";

    fn typed() -> ExitRequest {
        ExitRequest {
            admin_password: String::new(),
            code: None,
            destination: Some(TYPED.into()),
            understood: false,
        }
    }

    #[test]
    fn leftover_money_never_goes_to_a_stale_address() {
        // A withdrawal was stopped earlier: its address is still saved, but
        // the address typed now is the one used.
        let d = leftover_destination(&typed(), Some(OLD.into()), false, "test").unwrap();
        assert_eq!(d, TYPED);
        // While a withdrawal is on record, its own address binds.
        let d = leftover_destination(&typed(), Some(OLD.into()), true, "test").unwrap();
        assert_eq!(d, OLD);
    }

    #[tokio::test]
    async fn emergency_withdrawal_movements_are_kept_with_the_transfers() {
        let db = crate::db::Db::open(":memory:").unwrap();
        db.call(|conn| {
            record_exit_movement(conn, "exit_start", 40_000, OLD, "started")?;
            record_exit_movement(conn, "exit_claim", 0, OLD, "claimed, transaction abc")?;
            record_exit_movement(conn, "exit_leftover", -5, OLD, "sent back")?;
            let rows: Vec<(String, i64, String, String)> = conn
                .prepare(
                    "SELECT kind, amount_sats, destination, status FROM settlements ORDER BY rowid",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<_, _>>()?;
            assert_eq!(
                rows,
                vec![
                    ("exit_start".into(), 40_000, OLD.into(), "broadcast".into()),
                    ("exit_claim".into(), 0, OLD.into(), "broadcast".into()),
                    ("exit_leftover".into(), 0, OLD.into(), "broadcast".into()),
                ]
            );
            // They are not counted as transfers to the payout wallet, and never
            // look like an unclear send.
            assert!(crate::wallet::settlement::unsettled_send(conn)?.is_none());
            Ok(())
        })
        .await
        .unwrap();
    }
}
