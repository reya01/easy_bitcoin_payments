//! Payout address endpoints (see `crate::payout`).
//!
//! * `GET  /api/v1/payout`         — current / pending address (signed in).
//! * `POST /admin/payout`          — change it (session + CSRF + admin password).
//! * `POST /admin/payout/min`      — change the smallest amount that is sent on
//!   (one for Ark, one for on-chain; each has a hard floor; same protection).
//! * `POST /admin/payout/cancel`   — cancel a pending change (session + CSRF).
//! * `POST /admin/sweep`           — move the whole till to the payout address now
//!   (session + CSRF; it can only ever pay the saved payout address).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_auth as aa;
use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::routes::admin_session::require_session;
use crate::routes::AppState;

fn view(s: &crate::payout::Status, config: &crate::config::Config) -> Value {
    use crate::payout::Rail;
    let delay_secs = config.payout_change_delay_secs;
    let rail = s.active.as_deref().map(Rail::of);
    let kind = rail.map(|r| match r {
        Rail::Ark => "ark",
        Rail::Onchain => "onchain",
    });
    let (min_override, floor) = match rail {
        Some(Rail::Onchain) => (s.onchain_min_override, Rail::Onchain.floor()),
        _ => (s.ark_min_override, Rail::Ark.floor()),
    };
    json!({
        "address": s.active,
        "kind": kind,
        "sweep_min_sats": min_override.unwrap_or(floor),
        "sweep_min_floor_sats": floor,
        "sweep_min_is_custom": min_override.is_some(),
        "ark_min_floor_sats": Rail::Ark.floor(),
        "onchain_min_floor_sats": Rail::Onchain.floor(),
        "exit_opted_in": s.exit_opted_in,
        "exit_min_sats": crate::emergency_exit::MIN_EXIT_SATS,
        "small_coin_sats": crate::wallet::settlement::SMALL_COIN_SATS,
        "onchain_max_fee_sat_vb": config.onchain_sweep_max_fee_sat_vb,
        "pending_address": s.pending,
        "pending_effective_at": s.effective_at,
        "set_by_server_config": s.from_server_config,
        "change_delay_hours": delay_secs / 3600,
        "live": config.mode == "live",
    })
}

pub async fn get(State(state): State<AppState>, _auth: OwnerAuth) -> Result<Json<Value>, AppError> {
    let cfg = state.config.settlement_destination.clone();
    let s = state
        .db
        .call(move |c| crate::payout::status(c, cfg.as_deref()))
        .await?;
    Ok(Json(view(&s, &state.config)))
}

#[derive(Deserialize)]
pub struct ChangeRequest {
    pub address: String,
    pub admin_password: String,
    /// Authenticator code, needed only when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

pub async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChangeRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    // Validate first so a typo never costs a slow password check.
    let address = crate::payout::validate(&req.address, &state.config.mode)?;

    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let pw = req.admin_password;
    let ok = aa::verify_login_slow(stored, pw).await?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(&state, &headers, req.code).await?;

    let delay = state.config.payout_change_delay_secs;
    let cfg = state.config.settlement_destination.clone();
    let mode = state.config.mode.clone();
    let (change, s) = state
        .db
        .call(move |c| {
            let ch = crate::payout::request_change(c, &address, delay, cfg.as_deref(), &mode)?;
            let s = crate::payout::status(c, cfg.as_deref())?;
            Ok((ch, s))
        })
        .await?;
    let mut v = view(&s, &state.config);
    v["applied_now"] = json!(matches!(change, crate::payout::Change::Now));
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct MinRequest {
    /// Which kind of payout address the number is for: `ark` or `onchain`.
    pub kind: String,
    /// The smallest amount that is sent on, in sats (at least that kind's
    /// floor). Absent or null = go back to the default (the floor).
    #[serde(default)]
    pub min_sats: Option<i64>,
    pub admin_password: String,
    /// Authenticator code (or passkey approval), needed only when two-step sign-in is on.
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /admin/payout/min` — change the smallest amount that is sent on to the
/// payout address (when it is reached, the whole balance goes). Each kind of
/// address has a hard floor (Ark 1,500 sats, on-chain 50,000); under it the
/// answer says why, before the (slow) password check. Needs the admin password
/// and, when on, the second step. Takes effect right away: it cannot send
/// money anywhere new.
pub async fn change_min(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MinRequest>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let rail = crate::payout::Rail::parse(&req.kind)
        .ok_or_else(|| AppError::BadRequest("kind must be \"ark\" or \"onchain\"".into()))?;
    if let Some(v) = req.min_sats {
        crate::payout::check_min(rail, v)?;
    }
    let stored = state.db.call(|c| aa::stored_hash(c)).await?;
    let ok = aa::verify_login_slow(stored, req.admin_password).await?;
    if !ok {
        return Err(AppError::BadRequest(
            "that admin password isn't right".into(),
        ));
    }
    crate::routes::admin_session::check_second_step(&state, &headers, req.code).await?;

    let cfg = state.config.settlement_destination.clone();
    let min = req.min_sats;
    let s = state
        .db
        .call(move |c| {
            match min {
                Some(v) => crate::payout::set_min(c, rail, v)?,
                None => crate::payout::clear_min(c, rail)?,
            }
            crate::payout::status(c, cfg.as_deref())
        })
        .await?;
    Ok(Json(view(&s, &state.config)))
}

pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let cfg = state.config.settlement_destination.clone();
    let s = state
        .db
        .call(move |c| {
            crate::payout::cancel(c)?;
            crate::payout::status(c, cfg.as_deref())
        })
        .await?;
    Ok(Json(view(&s, &state.config)))
}

/// `POST /admin/sweep` — the merchant asks for the whole till to be sent to
/// the saved payout address right now (ignores the till cap and the on-chain
/// fee limit). It cannot redirect money: the destination is always the saved
/// payout address.
pub async fn sweep_now(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let cfg = state.config.settlement_destination.clone();
    let has_destination = state
        .db
        .call(move |c| crate::payout::effective(c, cfg.as_deref()))
        .await?
        .is_some_and(|d| !d.trim().is_empty());
    if !has_destination {
        return Err(AppError::BadRequest(
            "add a payout address first; there is nowhere to send the money yet".into(),
        ));
    }
    let client = crate::wallet::barkd::BarkdClient::from_app_config(&state.config)
        .map_err(AppError::from)?;
    let engine = crate::wallet::settlement::SettlementEngine::new(
        client,
        state.db.clone(),
        state.config.clone(),
    );
    let report = engine.sweep_now().await?;
    Ok(Json(json!({
        "swept_sats": report.swept_sats,
        "note": report.sweep_refused,
        // What could not be part of this send, and why (see HeldBack).
        "held_back": report
            .held_back
            .iter()
            .map(|h| json!({ "kind": h.kind, "sats": h.sats }))
            .collect::<Vec<_>>(),
    })))
}

/// Body of `POST /admin/transfer-outcome`.
#[derive(Deserialize)]
pub struct TransferOutcome {
    /// `arrived` or `not_arrived`.
    pub outcome: String,
}

/// `POST /admin/transfer-outcome` — the owner says how an interrupted transfer
/// ended, after checking their payout wallet. `not_arrived` lets sending resume.
/// It cannot send anything or change where money goes.
pub async fn transfer_outcome(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TransferOutcome>,
) -> Result<Json<Value>, AppError> {
    require_session(&state, &headers).await?;
    let arrived = match body.outcome.as_str() {
        "arrived" => true,
        "not_arrived" => false,
        _ => {
            return Err(AppError::BadRequest(
                "outcome must be arrived or not_arrived".into(),
            ))
        }
    };
    // "It did not arrive" lets the transfer be sent again, so it is checked
    // against the wallet's own history first: if the history shows it, saying it
    // did not arrive would send the money twice. If the history cannot be read
    // the answer is refused too ("try again in a minute"), never guessed.
    if !arrived {
        let client = crate::wallet::barkd::BarkdClient::from_app_config(&state.config)
            .map_err(AppError::from)?;
        let shows = crate::wallet::settlement::unsettled_shows_in_history(&client, &state.db)
            .await
            .map_err(|_| {
                AppError::BadRequest(
                    "The server could not read the wallet's history just now, so it cannot check your answer. Nothing was changed and nothing is sent. Try again in a minute.".into(),
                )
            })?;
        if shows == Some(true) {
            return Err(AppError::BadRequest(
                "The wallet's history shows this transfer, so it looks like it went out. Saying it did not arrive could send the money twice. Check your payout wallet again; if it is really not there, wait a few minutes and try again.".into(),
            ));
        }
    }
    let found = state
        .db
        .call(move |c| {
            let unclear = crate::wallet::settlement::unsettled_send_older_than(c, 120)?;
            let found = crate::wallet::settlement::resolve_unsettled(c, arrived)?;
            if found {
                let what = unclear
                    .map(|u| format!("{} {} sats ({})", u.kind, u.amount_sats, u.id))
                    .unwrap_or_default();
                crate::routes::twofa::audit(
                    c,
                    "transfer.outcome",
                    &format!(
                        "owner: {} - {what}",
                        if arrived { "arrived" } else { "not arrived" }
                    ),
                );
            }
            Ok(found)
        })
        .await?;
    if !found {
        return Err(AppError::BadRequest(
            "there is no unclear transfer to settle".into(),
        ));
    }
    Ok(Json(json!({ "ok": true })))
}
