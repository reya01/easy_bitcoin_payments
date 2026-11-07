use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::routes::AppState;
use crate::wallet::AspStatus;

/// How long the public health endpoint waits for the wallet before reporting
/// it unreachable. Shorter than the Docker healthcheck timeout (5 s).
const HEALTH_WALLET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Serialize)]
pub struct WalletHealthView {
    pub reachable: bool,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub database: &'static str,
    pub wallet: WalletHealthView,
    pub asp: &'static str,
    /// Seconds since the payment watcher last completed a tick; `null`
    /// until the first tick. A value much larger than the poll interval
    /// means the watcher is wedged or failing (beta hardening).
    pub watcher_last_tick_age_secs: Option<i64>,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let db_ok = state
        .db
        .call(|conn| Ok(conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?))
        .await
        .is_ok();

    // Wallet health blocks on HTTP for barkd; keep it off the async workers.
    let wallet_adapter = state.wallet.clone();
    // A hung barkd must not hang this public endpoint (the Docker healthcheck
    // and store probes give up after a few seconds): answer "not reachable"
    // instead. The blocking call finishes on its own at the adapter timeout.
    let wallet = match tokio::time::timeout(
        HEALTH_WALLET_TIMEOUT,
        tokio::task::spawn_blocking(move || wallet_adapter.health()),
    )
    .await
    {
        Ok(Ok(h)) => h,
        Ok(Err(_)) => crate::wallet::WalletHealth {
            reachable: false,
            balance_sats: None,
            asp: AspStatus::Unknown,
            detail: "wallet health check panicked".to_string(),
        },
        Err(_) => crate::wallet::WalletHealth {
            reachable: false,
            balance_sats: None,
            asp: AspStatus::Unknown,
            detail: "the wallet is not answering".to_string(),
        },
    };

    let overall = if db_ok && wallet.reachable {
        "ok"
    } else if db_ok {
        "degraded"
    } else {
        "down"
    };

    let last_tick = crate::watcher::LAST_TICK_UNIX.load(std::sync::atomic::Ordering::Relaxed);
    let watcher_last_tick_age_secs = (last_tick > 0).then(|| crate::util::now_unix() - last_tick);

    Json(HealthResponse {
        watcher_last_tick_age_secs,
        status: overall,
        version: crate::VERSION,
        database: if db_ok { "ok" } else { "down" },
        // Note: the till balance is deliberately NOT exposed here. This
        // endpoint is public (Docker healthcheck, store probes) and the spec
        // defines it as "server, wallet, ASP reachability" — broadcasting the
        // hot-float size tells attackers exactly how much is stealable.
        // Balances belong to the authenticated dashboard (Phase 1).
        wallet: WalletHealthView {
            reachable: wallet.reachable,
            detail: wallet.detail,
        },
        asp: match wallet.asp {
            AspStatus::Ok => "ok",
            AspStatus::Degraded => "degraded",
            AspStatus::Down => "down",
            AspStatus::Unknown => "unknown",
        },
    })
}
