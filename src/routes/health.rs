use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::routes::AppState;
use crate::wallet::AspStatus;

#[derive(Debug, Serialize)]
pub struct WalletHealthView {
    pub reachable: bool,
    pub balance_sats: Option<i64>,
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

    let wallet = state.wallet.health();

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
        wallet: WalletHealthView {
            reachable: wallet.reachable,
            balance_sats: wallet.balance_sats,
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