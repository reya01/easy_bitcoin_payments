use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::routes::AppState;
use crate::wallet::AspStatus;

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
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let db_ok = state
        .db
        .call(|conn| Ok(conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))?))
        .await
        .is_ok();

    // Wallet health blocks on HTTP for barkd; keep it off the async workers.
    let wallet_adapter = state.wallet.clone();
    let wallet = tokio::task::spawn_blocking(move || wallet_adapter.health())
        .await
        .unwrap_or_else(|_| crate::wallet::WalletHealth {
            reachable: false,
            balance_sats: None,
            asp: AspStatus::Unknown,
            detail: "wallet health check panicked".to_string(),
        });

    let overall = if db_ok && wallet.reachable {
        "ok"
    } else if db_ok {
        "degraded"
    } else {
        "down"
    };

    Json(HealthResponse {
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
