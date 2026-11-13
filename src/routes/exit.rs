//! Settings → "Emergency: withdraw without the Ark server" (MAINNET_PLAN
//! PR G). Every action needs a signed-in session, the admin password AND a
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
const EXIT_DESTINATION_KEY: &str = "exit_destination";

fn client(state: &AppState) -> Result<BarkdClient, AppError> {
    BarkdClient::from_app_config(&state.config).map_err(AppError::from)
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
    crate::routes::admin_session::check_second_step(state, code).await
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
    let (two_step_on, started_at, destination) = state
        .db
        .call(|conn| {
            Ok((
                crate::totp::is_enabled(conn)?,
                crate::settings::get(conn, ee::EXIT_STARTED_AT_KEY)?
                    .and_then(|v| v.parse::<i64>().ok()),
                crate::settings::get(conn, EXIT_DESTINATION_KEY)?,
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
    let fee = c
        .exit_fee(claim_to.as_deref())
        .await
        .ok()
        .as_ref()
        .and_then(ee::parse_fee);
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
    let refusal = match (ee::not_enabled_reason(&state.config.mode), ark_sats) {
        (Some(off), _) => Some(off.to_string()),
        (None, Some(a)) => ee::refusal(a, fee.as_ref()),
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
        "deposit_needed_sats": fee.map(|f| ee::deposit_still_needed(&f, confirmed.unwrap_or(0))),
        "deadline": deadline,
        "asp_connected": connected,
        "two_step_on": two_step_on,
        "allowed": refusal.is_none(),
        "refusal": refusal,
        "in_progress_since": started_at,
        "live": live_view,
        "claimable": ee::any_claimable(&live),
        "destination": claim_to,
    })))
}

/// `POST /admin/exit/deposit-address` — the server's own on-chain address,
/// where the owner sends the money that pays the exit fees.
pub async fn deposit_address(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let address = client(&state)?.onchain_address().await?;
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
    if let Some(off) = ee::not_enabled_reason(&state.config.mode) {
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
    let already = state
        .db
        .call(|c| crate::settings::get(c, ee::EXIT_STARTED_AT_KEY))
        .await?;
    if already.is_some() {
        return Err(AppError::BadRequest(
            "An emergency withdrawal is already in progress.".into(),
        ));
    }

    let c = client(&state)?;
    let ark_sats = c.balances().await?.offchain_sats;
    let fee = c
        .exit_fee(Some(&destination))
        .await
        .ok()
        .as_ref()
        .and_then(ee::parse_fee);
    if let Some(why) = ee::refusal(ark_sats, fee.as_ref()) {
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
            Ok(())
        })
        .await?;
    Ok(Json(json!({
        "started": true,
        "message": "Started. The server now sends the exit transactions by itself over the next hours. Leave the server on, and download a fresh backup now: the backup carries the withdrawal, the 12 words do not.",
    })))
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
                    "DELETE FROM settings WHERE key IN (?1, ?2)",
                    rusqlite::params![ee::EXIT_STARTED_AT_KEY, ee::EXIT_CLAIMABLE_KEY],
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
    let destination = destination_from(&req, saved, &state.config.mode)?;
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    let live = ee::parse_live(&c.exits_live().await?);
    if !ee::any_claimable(&live) {
        return Err(AppError::BadRequest(
            "Nothing is ready to claim yet. The page says when it is.".into(),
        ));
    }
    c.claim_exits(&destination).await?;
    let detail = format!("emergency withdrawal claimed to {}", short(&destination));
    state
        .db
        .call(move |conn| {
            conn.execute(
                "DELETE FROM settings WHERE key = ?1",
                rusqlite::params![ee::EXIT_CLAIMABLE_KEY],
            )?;
            audit(conn, "exit.claimed", &detail);
            Ok(())
        })
        .await?;
    Ok(Json(json!({
        "claimed": true,
        "message": "Sent. It arrives at your address once the transaction confirms. If any fee money is left on the server, send it back with the button below.",
    })))
}

/// `POST /admin/exit/leftover` — send what is left of the fee deposit back.
/// Refused while an exit is still running (it may need that money).
pub async fn leftover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ExitRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let saved = state
        .db
        .call(|c| crate::settings::get(c, EXIT_DESTINATION_KEY))
        .await?;
    let destination = destination_from(&req, saved, &state.config.mode)?;
    authorize(&state, &headers, req.admin_password, req.code).await?;
    let c = client(&state)?;
    let live = ee::parse_live(&c.exits_live().await?);
    if live.iter().any(|e| e.state != "claim-in-progress") {
        return Err(AppError::BadRequest(
            "Wait until the emergency withdrawal has finished: it may still need this money for fees.".into(),
        ));
    }
    let answer = c.onchain_drain(&destination).await?;
    let txid = answer
        .get("txid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let detail = format!("left-over on-chain money sent to {}", short(&destination));
    state
        .db
        .call(move |conn| {
            audit(conn, "exit.leftover_sent", &detail);
            Ok(())
        })
        .await?;
    Ok(Json(json!({
        "sent": true,
        "txid": txid,
        "message": "Sent the left-over on-chain money to your address.",
    })))
}
