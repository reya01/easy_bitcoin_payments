//! Settings, Your address: moving to the owner's own domain (in testing, behind
//! a switch). Every check and decision is in `crate::own_domain`; these routes
//! only call it. See docs/OWN_DOMAIN.md.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::OwnerAuth;
use crate::error::AppError;
use crate::own_domain::{self as od, Current, Finding, MoveState, Serve, Step};
use crate::routes::admin_session::check_second_step;
use crate::routes::AppState;
use crate::util::now_unix;

const OFF: &str = "Turn on \"Use your own address\" in Settings first.";
const NO_HELPER: &str = "This server cannot change its address from the admin page yet. Run `sudo ./scripts/install-update-helper.sh` once in the Console (it installs the address helper too), or follow docs/OWN_DOMAIN.md.";

fn current(state: &AppState) -> Current {
    Current::from_config(&state.config)
}

/// The server's public IPv4 address, when its address tells us (an sslip.io name).
fn server_ip(cur: &Current) -> Option<std::net::IpAddr> {
    cur.main
        .iter()
        .chain(cur.backups.iter())
        .find_map(|h| od::sslip_ip(h))
}

/// `pay.` plus the shop's own domain, when the shop's notice address tells us.
fn suggestion(webhook_url: Option<&str>) -> Option<String> {
    let host = od::host_of(webhook_url?)?;
    let base = host.strip_prefix("www.").unwrap_or(&host);
    let s = format!("pay.{base}");
    od::clean_name(&s).ok()
}

async fn load_state(state: &AppState) -> Result<(bool, MoveState), AppError> {
    let cur = current(state);
    state
        .db
        .call(move |c| {
            let mut st = od::load(c)?;
            let before = st.clone();
            od::reconcile(&mut st, &cur, now_unix());
            if st != before {
                od::save(c, &st)?;
            }
            Ok((od::is_enabled(c)?, st))
        })
        .await
}

fn view(
    state: &AppState,
    enabled: bool,
    st: &MoveState,
    shop_host: Option<String>,
    webhook: Option<String>,
) -> Value {
    let cur = current(state);
    let now = now_unix();
    let record_name = st
        .name
        .as_deref()
        .map(|n| n.split('.').next().unwrap_or(n).to_string());
    json!({
        "enabled": enabled,
        "helper_installed": od::helper_installed(),
        "main": cur.main,
        "backups": cur.backups,
        "server_ip": server_ip(&cur).map(|ip| ip.to_string()),
        "suggestion": suggestion(webhook.as_deref()),
        "step": st.step,
        "name": st.name,
        "record": {
            "type": "A",
            "name": record_name,
            "value": server_ip(&cur).map(|ip| ip.to_string()),
            "proxy": "DNS only",
        },
        "switched_at": st.switched_at,
        "days_left": od::days_left(st, now).map(|(w, r)| json!({"watch": w, "backup": r})),
        "failures": st.failures,
        "last_problem": st.last_problem,
        "last_ok_at": st.last_ok_at,
        "shop_host": shop_host,
        "shop_moved": match (&shop_host, &st.name) { (Some(h), Some(n)) => Some(h == n), _ => None },
        "keep_backup_until": st.keep_backup_until,
    })
}

/// `GET /api/v1/admin/address`
pub async fn get_address(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<Value>, AppError> {
    let (enabled, st) = load_state(&state).await?;
    let shop_host = state
        .db
        .call(|c| crate::settings::get(c, od::SHOP_HOST_KEY))
        .await?;
    Ok(Json(view(
        &state,
        enabled,
        &st,
        shop_host,
        auth.store.webhook_url.clone(),
    )))
}

#[derive(Deserialize)]
pub struct EnabledReq {
    pub enabled: bool,
}

/// `POST /api/v1/admin/address/enabled`
pub async fn set_enabled(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Json(req): Json<EnabledReq>,
) -> Result<Json<Value>, AppError> {
    let (store_id, on) = (auth.store.id.clone(), req.enabled);
    state
        .db
        .call(move |c| {
            crate::settings::set(c, od::ENABLED_KEY, if on { "1" } else { "0" })?;
            crate::audit::record(
                c,
                &store_id,
                if on {
                    "address.enabled"
                } else {
                    "address.disabled"
                },
                None,
            )
        })
        .await?;
    Ok(Json(json!({ "enabled": on })))
}

async fn require_on(state: &AppState) -> Result<MoveState, AppError> {
    let (enabled, st) = load_state(state).await?;
    if !enabled {
        return Err(AppError::BadRequest(OFF.into()));
    }
    Ok(st)
}

async fn store_state(state: &AppState, st: MoveState) -> Result<(), AppError> {
    state.db.call(move |c| od::save(c, &st)).await
}

#[derive(Deserialize)]
pub struct CheckReq {
    pub name: String,
}

/// `POST /api/v1/admin/address/check` — does this name reach this server yet?
/// Changes nothing on the web server.
pub async fn check(
    State(state): State<AppState>,
    auth: OwnerAuth,
    Json(req): Json<CheckReq>,
) -> Result<Json<Value>, AppError> {
    let mut st = require_on(&state).await?;
    if matches!(
        st.step,
        Step::Main
            | Step::MainRequested
            | Step::RetireRequested
            | Step::Retired
            | Step::BackRequested
    ) {
        return Err(AppError::BadRequest(
            "A move is already done or in progress. Use Go back first to start over.".into(),
        ));
    }
    let name = od::clean_name(&req.name)?;
    let cur = current(&state);
    if cur.main.as_deref() == Some(name.as_str()) {
        return Err(AppError::BadRequest(
            "That is already this server's main address.".into(),
        ));
    }
    let now = now_unix();
    if st.name.as_deref() != Some(name.as_str()) {
        st = MoveState {
            name: Some(name.clone()),
            ..MoveState::default()
        };
    }
    let token = od::new_token(&mut st, now);
    store_state(&state, st.clone()).await?;
    let finding = od::precheck(&name, &token, server_ip(&cur)).await;
    if finding == Finding::Ready && st.step == Step::Idle {
        st.step = Step::Checked;
    }
    let words = finding.words();
    let (store_id, detail) = (auth.store.id.clone(), format!("{name}: {words}"));
    state
        .db
        .call(move |c| {
            od::save(c, &st)?;
            crate::audit::record(c, &store_id, "address.checked", Some(&detail))
        })
        .await?;
    Ok(Json(json!({ "finding": finding, "words": words })))
}

/// `POST /api/v1/admin/address/serve` — serve the checked name next to the
/// current address and get its certificate. The current address keeps working.
pub async fn serve(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<Value>, AppError> {
    let mut st = require_on(&state).await?;
    let Some(name) = st.name.clone().filter(|_| st.step == Step::Checked) else {
        return Err(AppError::BadRequest("Check the name first.".into()));
    };
    if !od::helper_installed() {
        return Err(AppError::BadRequest(NO_HELPER.into()));
    }
    let cur = current(&state);
    // Check again right now: DNS may have changed since.
    let token = od::new_token(&mut st, now_unix());
    store_state(&state, st.clone()).await?;
    let finding = od::precheck(&name, &token, server_ip(&cur)).await;
    if finding != Finding::Ready {
        st.step = Step::Idle;
        store_state(&state, st).await?;
        return Ok(Json(
            json!({ "finding": finding, "words": finding.words() }),
        ));
    }
    let main = cur
        .main
        .clone()
        .ok_or_else(|| AppError::Internal("no main address".into()))?;
    let mut others = cur.backups.clone();
    others.retain(|b| b != &name);
    others.push(name.clone());
    let id = od::request_serve(&Serve { main, others })?;
    st.step = Step::ServeRequested;
    st.request_id = Some(id);
    let (store_id, detail) = (auth.store.id.clone(), name.clone());
    state
        .db
        .call(move |c| {
            od::save(c, &st)?;
            crate::audit::record(c, &store_id, "address.serve_requested", Some(&detail))
        })
        .await?;
    Ok(Json(json!({ "requested": true })))
}

#[derive(Deserialize)]
pub struct CodeReq {
    #[serde(default)]
    pub code: Option<String>,
}

/// `POST /api/v1/admin/address/make-main` — new links use the new name; the old
/// address stays as the backup. Protected like a payout change.
pub async fn make_main(
    State(state): State<AppState>,
    headers: HeaderMap,
    auth: OwnerAuth,
    Json(req): Json<CodeReq>,
) -> Result<Json<Value>, AppError> {
    let mut st = require_on(&state).await?;
    let Some(name) = st.name.clone().filter(|_| st.step == Step::Serving) else {
        return Err(AppError::BadRequest(
            "The new name is not being served yet.".into(),
        ));
    };
    check_second_step(&state, &headers, req.code).await?;
    let token = od::new_token(&mut st, now_unix());
    store_state(&state, st.clone()).await?;
    if !od::token_answers("https", &name, &token).await {
        let f = Finding::NoCertificate;
        return Ok(Json(json!({ "finding": f, "words": f.words() })));
    }
    let cur = current(&state);
    let old_main = cur
        .main
        .clone()
        .ok_or_else(|| AppError::Internal("no main address".into()))?;
    let mut others = vec![old_main.clone()];
    others.extend(
        cur.backups
            .iter()
            .filter(|b| **b != name && **b != old_main)
            .cloned(),
    );
    let id = od::request_serve(&Serve {
        main: name.clone(),
        others,
    })?;
    st.step = Step::MainRequested;
    st.backup = Some(old_main);
    st.request_id = Some(id);
    let (store_id, detail) = (auth.store.id.clone(), name);
    state
        .db
        .call(move |c| {
            od::save(c, &st)?;
            crate::audit::record(c, &store_id, "address.main_requested", Some(&detail))
        })
        .await?;
    Ok(Json(json!({ "requested": true })))
}

/// `POST /api/v1/admin/address/go-back` — the backup becomes the main address
/// again. Always allowed while there is a backup; nothing about DNS changes.
pub async fn go_back(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<Value>, AppError> {
    let (_, mut st) = load_state(&state).await?;
    let (Some(name), Some(backup)) = (st.name.clone(), st.backup.clone()) else {
        return Err(AppError::BadRequest(
            "There is no backup address to go back to.".into(),
        ));
    };
    if !matches!(st.step, Step::Main | Step::MainRequested) {
        return Err(AppError::BadRequest(
            "Your own address is not the main address, so there is nothing to go back from.".into(),
        ));
    }
    let id = od::request_serve(&Serve {
        main: backup.clone(),
        others: vec![name.clone()],
    })?;
    st.step = Step::BackRequested;
    st.request_id = Some(id);
    let (store_id, detail) = (auth.store.id.clone(), format!("{name} -> {backup}"));
    state
        .db
        .call(move |c| {
            od::save(c, &st)?;
            crate::audit::record(c, &store_id, "address.back_requested", Some(&detail))
        })
        .await?;
    Ok(Json(json!({ "requested": true })))
}

/// `POST /api/v1/admin/address/keep-backup` — postpone giving up the backup.
pub async fn keep_backup(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<Value>, AppError> {
    let (_, mut st) = load_state(&state).await?;
    let Some(switched) = st.switched_at.filter(|_| st.step == Step::Main) else {
        return Err(AppError::BadRequest(
            "There is no backup waiting to be given up.".into(),
        ));
    };
    let now = now_unix();
    let base = st
        .keep_backup_until
        .unwrap_or(0)
        .max(now)
        .max(switched + od::RETIRE_DAYS * 86_400);
    st.keep_backup_until = Some(base + od::POSTPONE_DAYS * 86_400);
    st.told.retain(|t| t != "own_address_backup_soon");
    let until = st.keep_backup_until;
    let store_id = auth.store.id.clone();
    state
        .db
        .call(move |c| {
            od::save(c, &st)?;
            crate::audit::record(c, &store_id, "address.backup_kept", None)
        })
        .await?;
    Ok(Json(json!({ "keep_backup_until": until })))
}

/// `POST /api/v1/admin/address/cancel` — stop a move before the switch; the new
/// name is no longer served. Nothing changes for the main address.
pub async fn cancel(
    State(state): State<AppState>,
    auth: OwnerAuth,
) -> Result<Json<Value>, AppError> {
    let (_, st) = load_state(&state).await?;
    if matches!(
        st.step,
        Step::Main
            | Step::MainRequested
            | Step::RetireRequested
            | Step::Retired
            | Step::BackRequested
    ) {
        return Err(AppError::BadRequest(
            "The new name is already the main address. Use Go back instead.".into(),
        ));
    }
    let cur = current(&state);
    if let (Some(name), Some(main)) = (st.name.clone(), cur.main.clone()) {
        if cur.backups.contains(&name) && od::helper_installed() {
            let others = cur
                .backups
                .iter()
                .filter(|b| **b != name)
                .cloned()
                .collect();
            od::request_serve(&Serve { main, others })?;
        }
    }
    let store_id = auth.store.id.clone();
    state
        .db
        .call(move |c| {
            od::save(c, &MoveState::default())?;
            crate::audit::record(c, &store_id, "address.cancelled", None)
        })
        .await?;
    Ok(Json(json!({ "cancelled": true })))
}

/// `GET /.well-known/ebp-check/{token}` — answers with the token while a check
/// is running, so the server can tell a name really reaches it. Public; the
/// token is random, single purpose and expires in 30 minutes.
pub async fn check_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    if token.len() != 32 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let given = token.clone();
    let ok = state
        .db
        .call(move |c| Ok(od::token_matches(&od::load(c)?, &given, now_unix())))
        .await
        .unwrap_or(false);
    if ok {
        (StatusCode::OK, [("cache-control", "no-store")], token).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_suggested_name_is_pay_dot_the_shop_domain() {
        assert_eq!(
            suggestion(Some("https://www.myshop.com/?wc-api=ebp")).as_deref(),
            Some("pay.myshop.com")
        );
        assert_eq!(
            suggestion(Some("https://shop.example.co.uk/hook")).as_deref(),
            Some("pay.shop.example.co.uk")
        );
        assert_eq!(
            suggestion(Some("http://203.0.113.9/hook")),
            None,
            "an IP has no domain to suggest"
        );
        assert_eq!(suggestion(None), None);
    }
}
