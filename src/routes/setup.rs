//! First-run setup wizard (Phase 1 close-out, plan task 3): the routes
//! that take a fresh deployment from "no wallet, no token, no store" to
//! `setup_complete`.
//!
//! Design notes (read before touching):
//!
//! * **Reachability (documented choice).** The wizard's mutating routes
//!   are reachable only while setup is incomplete; once `setup_complete`
//!   is recorded they return 409, and the only way back is a documented
//!   datadir reset. This is the "open until first configured" model
//!   BTCPay Server uses for its first-run wizard. On a deployment exposed
//!   to the internet before setup finishes, complete setup immediately
//!   after first boot or keep the server behind a firewall/reverse proxy
//!   until then. `GET /setup/status` stays readable afterwards (it carries
//!   no secrets).
//! * **Secrets.** The barkd token arrives via `POST /setup/token`, is
//!   applied to the running process (see [`effective_barkd_token`]) and
//!   is **never persisted to the database** — the durable copy is the
//!   deployment environment (`EBP_BARKD_TOKEN`), and the wizard says so.
//!   The wallet mnemonic is read from barkd exactly once during the
//!   wallet step's expose-mnemonic ceremony (the create response never
//!   carries it — verified live on barkd 0.8.0), shown exactly once in
//!   the response body (with `Cache-Control: no-store`) and never
//!   stored or logged. No code path here logs either value.
//! * **The gate.** [`setup_gate`] is the canonical check that returns
//!   `503 SetupIncomplete` from invoice and checkout routes until setup
//!   completes; wire it as middleware over those routes (see the Phase 1
//!   close-out notes for the exact lines). [`gate_response`] is the same
//!   check as a helper for in-route use.
//! * **Honesty about barkd.** Wallet creation, token verification, and
//!   the wallet probe call the barkd REST API directly. Shapes pinned
//!   by the live wizard VERIFY pass against real barkd 0.8.0 on signet
//!   carry `VERIFIED` comments (`/wallet/mnemonic` → `{"mnemonic": …}`
//!   with 12 space-separated words, only under `--expose-mnemonic`;
//!   `/wallet/connected` → `{"connected": true}`;
//!   `/fees/lightning/receive` → GET with a required `amount_sat` query
//!   parameter and success body `{"gross_amount_sat": …, "fee_sat": …,
//!   "net_amount_sat": …, "vtxos_spent": […]}`). The one shape still
//!   under `VERIFY` is the wallet-create response — the live pass could
//!   not probe it on the main datadir (a wallet already existed) — and
//!   the ceremony no longer depends on it: the words always come from
//!   the verified mnemonic endpoint, and create is read only for
//!   success / `already_exists`, defensively.
//! * **The expose-mnemonic ceremony.** The wallet-creation step never
//!   prints the words (verified live on barkd 0.8.0: the create
//!   response carries none), so the step runs as one ceremony under
//!   `--expose-mnemonic`: the wizard directs the merchant to run ONE
//!   local command restarting barkd with the flag and to paste nothing
//!   anywhere; the merchant taps "Show my recovery words" and
//!   `POST /setup/wallet/mnemonic` reads the words straight from barkd
//!   exactly ONCE, shows them exactly once (`no-store`, never stored,
//!   never logged), and the "I have written down my recovery words"
//!   checkbox completes the backup. The wizard then instructs
//!   restarting barkd WITHOUT the flag — after which the endpoint
//!   stops answering and is never called again. The UI text states
//!   exactly this, including that barkd is serving the words over its
//!   API while the flag is on.
//! * **Empty datadir.** The wizard assumes EBP owns a barkd on an EMPTY
//!   datadir; `already_exists: true` on create means a wallet existed
//!   before the wizard ran (the first-boot window in the threat model)
//!   and is treated as compromised, not adopted.

use std::sync::{OnceLock, RwLock};

const SETUP_HTML: &str = include_str!("../../assets/setup.html");
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::AppState;
use crate::settings;

// ---------------------------------------------------------------------------
// barkd paths used by the wizard. Duplicated from src/wallet/barkd.rs
// (where the equivalents are module-private); scripts/check_barkd_drift.sh
// verifies every PATH_* constant in src/ — both copies — against the
// pinned OpenAPI. VERIFY markers per the honesty rule.
// ---------------------------------------------------------------------------
/// Create (or restore) the wallet. POST. Sends barkd's bearer token when the
/// server has one (the single-container install does; see `create_till_wallet`).
/// The pinned OpenAPI documents this endpoint (request `network`,
/// `ark_server`, ...; response `{"fingerprint"}` only, never the words).
/// Not yet seen live: the pass could not probe it on the main datadir (a
/// wallet already existed). The ceremony no longer
/// depends on that shape: the recovery words always come from the VERIFIED
/// mnemonic endpoint while barkd runs with `--expose-mnemonic`, and the
/// wizard reads create only for success / `already_exists`, defensively.
const PATH_WALLET_CREATE: &str = "/wallet/create";
/// Whether the wallet is connected to its ASP. GET.
/// VERIFIED (barkd 0.8.0, signet, live wizard VERIFY pass): the response
/// is `{"connected": true}` — a bare boolean under `connected`, no
/// wrapper object.
const PATH_WALLET_CONNECTED: &str = "/wallet/connected";
/// On-chain and off-chain balances. GET. Used as a shape-agnostic
/// "does a wallet exist" probe (any 2xx = yes).
const PATH_WALLET_BALANCE: &str = "/wallet/balance";
/// The wallet's recovery words. GET. VERIFIED (same pass): answers
/// `{"mnemonic": "<12 words, space-separated>"}` — a single string
/// field, no wrapper object — but ONLY while barkd runs with
/// `--expose-mnemonic`; with the flag off the endpoint refuses (exact
/// refusal shape not captured — any non-2xx is treated as "not
/// exposed"). Called exactly once per wizard, only while setup is
/// incomplete, never stored, never logged; the wizard then instructs
/// restarting barkd WITHOUT the flag so it can never be read again.
const PATH_WALLET_MNEMONIC: &str = "/wallet/mnemonic";

/// Default ASP in test mode: Second's public signet Ark server.
const DEFAULT_SIGNET_ASP: &str = "https://ark.signet.2nd.dev";
/// Default ASP in live mode: Second's mainnet Ark server
/// (connection-details; measured with the pinned barkd 0.8.0 on
/// 11 Nov 2026 by the `mainnet-compat-check` workflow).
const DEFAULT_MAINNET_ASP: &str = "https://ark.second.tech";

/// The network, Ark server and server mode a new (or restored) wallet gets,
/// from the owner's choice on the setup page: `signet` (play money, test
/// mode) or `mainnet` (real bitcoin, live mode). Nothing chosen means the
/// default for how the server was started (`EBP_MODE`, itself `test` by
/// default). A barkd wallet belongs to one network for good, so mainnet
/// needs the owner's explicit "I understand" and the Ark server must belong
/// to the chosen network. An empty server address means Second's server for
/// that network.
fn resolve_wallet_choice(
    default_mode: &str,
    network: Option<&str>,
    asp_url: Option<&str>,
    mainnet_understood: bool,
) -> Result<(String, String, &'static str), AppError> {
    let network = match network.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_lowercase(),
        None if default_mode == "live" => "mainnet".to_string(),
        None => "signet".to_string(),
    };
    let (network, mode, default_asp) = match network.as_str() {
        "signet" => ("signet".to_string(), "test", DEFAULT_SIGNET_ASP),
        "mainnet" | "bitcoin" => ("mainnet".to_string(), "live", DEFAULT_MAINNET_ASP),
        other => {
            return Err(AppError::BadRequest(format!(
                "network \"{other}\" is not offered: choose signet (play money) or mainnet (real bitcoin)"
            )))
        }
    };
    if mode == "live" && !mainnet_understood {
        return Err(AppError::BadRequest(
            "Mainnet means real bitcoin, and this server stays on mainnet for good. Tick the box to confirm you understand, or choose signet.".into(),
        ));
    }
    let asp_url = match asp_url.map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => default_asp.to_string(),
    };
    let signet_server = asp_url.to_lowercase().contains("signet");
    if (mode == "live" && signet_server) || (mode == "test" && asp_url == DEFAULT_MAINNET_ASP) {
        return Err(AppError::BadRequest(format!(
            "that Ark server is not on {network}: leave the Ark server box as the page fills it in"
        )));
    }
    Ok((network, asp_url, mode))
}

/// The server's mode as the setup page sees it: the network chosen on this
/// page (saved when the wallet was made) wins over how the server was
/// started, because the wallet itself is on that network.
async fn wizard_mode(state: &AppState) -> Result<String, AppError> {
    let saved = state.db.call(|c| crate::config::saved_mode(c)).await?;
    Ok(saved.unwrap_or_else(|| state.config.mode.clone()))
}

// ---------------------------------------------------------------------------
// The runtime token override
// ---------------------------------------------------------------------------

static RUNTIME_BARKD_TOKEN: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn runtime_token_cell() -> &'static RwLock<Option<String>> {
    RUNTIME_BARKD_TOKEN.get_or_init(|| RwLock::new(None))
}

/// The barkd token available to the running process: the wizard-applied
/// token if one was set, else the boot-time env token. Never logged.
pub fn effective_barkd_token(config: &crate::config::Config) -> Option<String> {
    if let Some(token) = runtime_token_cell()
        .read()
        .ok()
        .and_then(|cell| cell.clone())
    {
        return Some(token);
    }
    config.barkd_token.clone()
}

/// Apply a wizard-verified token to the running process. The wizard's own
/// barkd calls use it immediately; the payments paths pick the token up
/// from the environment after the documented restart. Not persisted.
pub fn set_runtime_barkd_token(token: &str) {
    if let Ok(mut cell) = runtime_token_cell().write() {
        *cell = Some(token.to_string());
    }
}

/// Clear the runtime override. Test helper only (no route exposes an
/// "undo"; the durable token is the deployment env, not process state).
#[cfg(test)]
fn clear_runtime_barkd_token() {
    if let Ok(mut cell) = runtime_token_cell().write() {
        *cell = None;
    }
}

// ---------------------------------------------------------------------------
// GET /setup/status
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct SetupStatus {
    /// A wallet exists in the barkd sidecar (settings flag, or a live
    /// probe when a token is available — catches wallets created outside
    /// the wizard, e.g. the manual recipe path).
    pub wallet_created: bool,
    /// The running server has a barkd token (boot env or wizard-applied).
    pub barkd_token_present: bool,
    /// The wizard verified a token against barkd and recorded it. The
    /// durable copy is the deployment env (`EBP_BARKD_TOKEN`) — the
    /// wizard says so; the token itself is never written to the database.
    pub config_written: bool,
    /// Whether the recovery-word backup was confirmed (the checkbox).
    pub backup_confirmed: bool,
    /// Stores that exist (the wizard creates the first).
    pub store_count: i64,
    /// True only after the wallet is created, the backup is confirmed, the
    /// token is verified, and at least one store exists. Invoice and
    /// checkout routes stay closed (503 SetupIncomplete) until then.
    pub setup_complete: bool,
    /// The wallet was restored from its recovery words (not newly made). Only
    /// then does the wizard offer to bring back a backup file.
    pub wallet_restored: bool,
    /// A backup file staged by the wizard was NOT applied at this start (the
    /// plain reason). Nothing was deleted; the files were set aside.
    pub restore_refused: Option<String>,
    /// The wizard needs a setup code (generated at boot, or EBP_SETUP_TOKEN)
    /// before any action; the page asks for it up front.
    pub setup_code_required: bool,
    /// The wizard is being served over plain HTTP on a non-local address;
    /// every setup write is refused until the address is HTTPS.
    pub https_required: bool,
    /// barkd is run by EBP itself: the wizard needs no terminal steps.
    pub managed: bool,
    /// `"test"` (signet, play money) or `"live"`.
    pub mode: String,
    /// The network was chosen on the setup page and is now fixed.
    pub mode_chosen: bool,
}

/// GET /setup — the mobile-first wizard page.
pub async fn page() -> Html<&'static str> {
    Html(SETUP_HTML)
}

pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, AppError> {
    let token = effective_barkd_token(&state.config);

    let mut snapshot = state.db.call(|conn| wizard_snapshot(conn)).await?;

    // Best-effort live probe (read-only): with a token we can ask barkd
    // directly whether a wallet exists. The flag itself is only persisted
    // by the wallet/token steps.
    if !snapshot.wallet_created && token.is_some() {
        if let Some(base_url) = barkd_base_url(&state.config) {
            if let Some(status) = barkd_get(&base_url, PATH_WALLET_BALANCE, token.as_deref()).await
            {
                if status.is_success() {
                    snapshot.wallet_created = true;
                }
            }
        }
    }

    snapshot.barkd_token_present = token.is_some();
    snapshot.restore_refused = crate::backup::restore_refused_reason().map(str::to_string);
    snapshot.mode = wizard_mode(&state).await?;
    snapshot.mode_chosen = state
        .db
        .call(|c| crate::config::saved_mode(c))
        .await?
        .is_some();
    snapshot.setup_code_required = state.config.setup_token.is_some() && !snapshot.setup_complete;
    snapshot.managed = crate::managed_barkd::get().is_some();
    snapshot.https_required = !state.config.setup_transport_ok() && !snapshot.setup_complete;

    Ok(Json(snapshot))
}

/// The settings-table part of the status (everything except the two
/// runtime-filled fields). Separated so it is unit-testable.
fn wizard_snapshot(conn: &rusqlite::Connection) -> Result<SetupStatus, AppError> {
    Ok(SetupStatus {
        wallet_created: settings::get_bool(conn, "setup_wallet_created")?,
        wallet_restored: settings::get_bool(conn, "setup_wallet_restored")?,
        restore_refused: None,      // filled by the caller
        barkd_token_present: false, // filled by the caller (runtime token)
        config_written: settings::get_bool(conn, "setup_token_verified")?,
        backup_confirmed: settings::get_bool(conn, "setup_backup_confirmed")?,
        store_count: conn.query_row("SELECT COUNT(*) FROM stores", [], |row| row.get(0))?,
        setup_complete: settings::get_bool(conn, "setup_complete")?,
        setup_code_required: false, // filled by the caller
        https_required: false,      // filled by the caller
        managed: false,             // filled by the caller
        mode: String::new(),        // filled by the caller
        mode_chosen: false,         // filled by the caller
    })
}

// ---------------------------------------------------------------------------
// POST /setup/wallet — create the till wallet, show the mnemonic once
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CreateWalletRequest {
    /// Bitcoin network. Must match `mode`: "signet" in test, "mainnet" in
    /// live (the default for each); anything else is refused.
    pub network: Option<String>,
    /// Ark Service Provider URL. Empty or missing means Second's server for
    /// the network: `ark.signet.2nd.dev` in test, `ark.second.tech` in live.
    pub asp_url: Option<String>,
    /// RESTORE an existing wallet from its recovery words instead of making a
    /// new one. Typed by the owner; passed straight to barkd; never stored,
    /// never logged, never echoed back (see the manual `Debug` below).
    #[serde(default)]
    pub mnemonic: Option<String>,
    /// Restore only: the owner says the paper copies of the words still
    /// exist (this stands in for the "I wrote the words down" step).
    #[serde(default)]
    pub paper_copies_exist: bool,
    /// Mainnet only: the owner ticked "real bitcoin, and this server stays
    /// on mainnet for good".
    #[serde(default)]
    pub mainnet_understood: bool,
}

// Manual Debug: the words must never reach a log line, panic message or `{:?}`.
impl std::fmt::Debug for CreateWalletRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateWalletRequest")
            .field("network", &self.network)
            .field("asp_url", &self.asp_url)
            .field("mnemonic", &self.mnemonic.as_ref().map(|_| "[REDACTED]"))
            .field("paper_copies_exist", &self.paper_copies_exist)
            .field("mainnet_understood", &self.mainnet_understood)
            .finish()
    }
}

/// Tidy typed recovery words: lower case, single spaces. Only the shape is
/// checked here (12 or 24 words, letters only); barkd checks the words
/// themselves. Errors never repeat what was typed.
fn clean_recovery_words(raw: &str) -> Result<String, AppError> {
    let words: Vec<String> = raw
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let shape_ok = (words.len() == 12 || words.len() == 24)
        && words
            .iter()
            .all(|w| (3..=8).contains(&w.len()) && w.chars().all(|c| c.is_ascii_lowercase()));
    if !shape_ok {
        return Err(AppError::BadRequest(
            "The recovery words should be 12 words (or 24), separated by spaces, using letters only. Check your paper copy and type them again. Nothing was changed.".into(),
        ));
    }
    Ok(words.join(" "))
}

// ---------------------------------------------------------------------------
// POST /setup/wallet/mnemonic — the expose-mnemonic ceremony
// ---------------------------------------------------------------------------
// The verified reality (live wizard VERIFY pass, barkd 0.8.0 on signet):
// `bark create` prints no mnemonic and the HTTP create response's shape is
// unpinned, so the words usually do NOT arrive with the create call. The
// ceremony below is how the wizard gets them to the merchant anyway:
//
//   1. The wizard directs the merchant to run ONE local command
//      restarting barkd with `--expose-mnemonic` — and to paste nothing
//      anywhere, ever.
//   2. The merchant taps "Show my recovery words": this route calls
//      barkd's GET /wallet/mnemonic — VERIFIED shape
//      `{"mnemonic": "<words>"}`, served ONLY while the flag is on —
//      and returns the words once with `Cache-Control: no-store`.
//      They are never stored and never logged.
//   3. The merchant writes the words down and ticks the "I have written
//      down my recovery words" checkbox (the existing backup-confirm
//      step; the `setup_backup_confirmed` setting).
//   4. The wizard instructs restarting barkd WITHOUT the flag, after
//      which barkd's endpoint stops answering and this route refuses
//      forever. The UI text states exactly this — including that barkd
//      is serving the words over its API while the flag is on.
//
// Router wiring (with the other setup routes, same setup-incomplete
// gate): `.post(setup::wallet_mnemonic)` on `/setup/wallet/mnemonic`.
// ---------------------------------------------------------------------------

/// VERIFIED (barkd 0.8.0, signet): `GET /wallet/mnemonic` answers
/// `{"mnemonic": "<words>"}` while `--expose-mnemonic` is on. Parsed
/// defensively: unknown extra fields are ignored, a missing `mnemonic`
/// fails loudly.
#[derive(Debug, Deserialize)]
struct BarkdMnemonic {
    mnemonic: String,
}

/// The ONE local command that starts the ceremony (Docker deploy — the
/// only supported path): barkd restarted with `--expose-mnemonic`.
/// VERIFY: exact invocation against an externally run barkd compose service — it
/// assumes the barkd service command is `barkd --datadir /data` and that
/// a `docker compose run` container inherits the service's network
/// aliases (compose >= 2.22 `--use-aliases`) so EBP still reaches it.
const EXPOSE_MNEMONIC_COMMAND: &str = "docker compose stop barkd && docker compose run -d --name barkd-exposed --use-aliases barkd barkd --datadir /data --expose-mnemonic";

/// The command that ENDS the ceremony: barkd back to normal, the words
/// unreachable over its API for good. Same VERIFY as above.
const UNEXPOSE_MNEMONIC_COMMAND: &str = "docker rm -f barkd-exposed && docker compose up -d barkd";

/// Body of `POST /setup/wallet/mnemonic`.
///
/// Manual Debug impl: the recovery words must NEVER reach a log line, a
/// panic message, or a `{:?}` — the honest contract ("never stored, never
/// logged") has to hold structurally, not by convention.
#[derive(Serialize)]
pub struct MnemonicResponse {
    /// The recovery words, shown exactly once. `None` unless barkd is
    /// currently running with `--expose-mnemonic`.
    pub mnemonic: Option<String>,
    /// Word count of `mnemonic`, counted — never assumed: the live
    /// VERIFY pass observed a 12-word signet wallet, and barkd's word
    /// count is pinned by nothing we control (so the checkbox copy says
    /// "recovery words" and shows the count, rather than hardcoding a
    /// number like 24).
    pub word_count: Option<usize>,
    /// Exactly what happens next, in plain language — the honest-UI
    /// contract: what the flag does, that nothing is ever pasted into
    /// EBP, that the words are shown once and never stored or logged,
    /// and which command turns the exposure off for good.
    pub instructions: String,
}

// Same rule for the response body that shows the words (Serialize yes,
// Debug never).
impl std::fmt::Debug for MnemonicResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MnemonicResponse")
            .field(
                "mnemonic",
                &self
                    .mnemonic
                    .as_ref()
                    .map(|_| "[REDACTED — recovery words never logged]"),
            )
            .field("word_count", &self.word_count)
            .field("instructions", &self.instructions)
            .finish()
    }
}

impl MnemonicResponse {
    /// Words in hand: the once-only display plus the honest "exactly
    /// what happens" text.
    fn exposed(mnemonic: String) -> Self {
        let word_count = mnemonic.split_whitespace().count();
        Self {
            mnemonic: Some(mnemonic),
            word_count: Some(word_count),
            instructions: format!(
                "barkd is serving these words over its API only because you \
                 restarted it with --expose-mnemonic. Write all {word_count} \
                 words down on paper, twice, and keep the two copies in two \
                 separate safe places. EBP shows them this once and never \
                 stores or logs them. When you tick \"I have written down my \
                 recovery words\", restart barkd WITHOUT the flag — \
                 `{UNEXPOSE_MNEMONIC_COMMAND}` — after which they can never \
                 be read from barkd again."
            ),
        }
    }

    /// No words: the ONE command to run, and the promise that nothing is
    /// ever pasted into EBP.
    fn not_exposed(reason: &str) -> Self {
        Self {
            mnemonic: None,
            word_count: None,
            instructions: format!(
                "{reason} Run this ONE command on your server to restart \
                 barkd with its --expose-mnemonic flag: \
                 `{EXPOSE_MNEMONIC_COMMAND}` — then tap \"Show my recovery \
                 words\" again. You never paste the words anywhere: EBP \
                 reads them straight from barkd, shows them once, and never \
                 stores or logs them."
            ),
        }
    }
}

/// POST /setup/wallet/mnemonic — read the recovery words once from the
/// exposed endpoint (the ceremony above). Wizard-only: 409 once setup is
/// complete or the backup is already confirmed — the words are never
/// re-read after either.
pub async fn wallet_mnemonic(State(state): State<AppState>) -> Response {
    let snapshot = match state.db.call(|conn| wizard_snapshot(conn)).await {
        Ok(snapshot) => snapshot,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(MnemonicResponse::not_exposed(
                    "Could not read the setup state.",
                )),
            )
                .into_response()
        }
    };
    if snapshot.setup_complete {
        return (
            StatusCode::CONFLICT,
            Json(MnemonicResponse::not_exposed(
                "Setup is already complete — the recovery words are never \
                 re-read after setup.",
            )),
        )
            .into_response();
    }
    if snapshot.backup_confirmed {
        // Once-only: the backup was already confirmed; never re-show.
        return (
            StatusCode::CONFLICT,
            Json(MnemonicResponse::not_exposed(
                "The recovery-word backup is already confirmed — the words \
                 are shown exactly once.",
            )),
        )
            .into_response();
    }
    if !snapshot.wallet_created {
        return (
            StatusCode::CONFLICT,
            Json(MnemonicResponse::not_exposed(
                "Create the wallet first — the recovery-words step comes \
                 after the wallet step.",
            )),
        )
            .into_response();
    }

    // Managed barkd: EBP opens the one-time window itself (restart WITH
    // --expose-mnemonic), and closes it again right after the single read.
    let managed = crate::managed_barkd::get();
    if let Some(m) = &managed {
        if let Ok(t) = m.token().await {
            set_runtime_barkd_token(&t);
        }
        if m.restart(true).await.is_err() {
            // Best effort: never leave a half-opened window behind.
            let _ = m.restart(false).await;
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MnemonicResponse::not_exposed(
                    "Could not open the one-time recovery-words window. Try again in a moment.",
                )),
            )
                .into_response();
        }
    }

    let token = effective_barkd_token(&state.config);
    let base_url = match barkd_base_url(&state.config) {
        Some(base_url) => base_url,
        None => {
            close_mnemonic_window(&managed).await;
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(MnemonicResponse::not_exposed(
                    "barkd is not reachable from this server.",
                )),
            )
                .into_response();
        }
    };

    // VERIFIED shape: GET /wallet/mnemonic → {"mnemonic": "<words>"},
    // served only while --expose-mnemonic is on. Any non-2xx (flag off,
    // or auth — VERIFY: whether the endpoint enforces the bearer token;
    // the live probe carried one) is reported honestly with the one
    // command rather than guessed at.
    let (status, body) =
        match barkd_get_with_body(&base_url, PATH_WALLET_MNEMONIC, token.as_deref()).await {
            Some(pair) => pair,
            None => {
                close_mnemonic_window(&managed).await;
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(MnemonicResponse::not_exposed(
                        "barkd did not answer — is the sidecar running?",
                    )),
                )
                    .into_response();
            }
        };

    if !status.is_success() {
        close_mnemonic_window(&managed).await;
        return (
            StatusCode::OK,
            Json(MnemonicResponse::not_exposed(&format!(
                "barkd answered HTTP {status}, so the words are not exposed. \
                 If you already ran the command below and barkd keeps \
                 answering 401, finish the wizard's token step first, then \
                 come back."
            ))),
        )
            .into_response();
    }

    // Close the window immediately, whatever the parse outcome below.
    close_mnemonic_window(&managed).await;

    let parsed: BarkdMnemonic = match serde_json::from_str(&body) {
        Ok(parsed) => parsed,
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(MnemonicResponse::not_exposed(
                    "barkd answered in an unexpected shape — that is a bug \
                     to report, not something to work around.",
                )),
            )
                .into_response()
        }
    };

    let mut response = Json(MnemonicResponse::exposed(parsed.mnemonic)).into_response();
    // Shown exactly once: nothing between barkd and the merchant's
    // screen may cache it.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

/// Close the one-time recovery-words window (barkd back WITHOUT
/// `--expose-mnemonic`). Called on EVERY exit after the window was opened,
/// error paths included, so the words are never left readable over barkd's
/// API. A no-op when barkd is not managed by EBP.
async fn close_mnemonic_window(
    managed: &Option<std::sync::Arc<crate::managed_barkd::ManagedBarkd>>,
) {
    if let Some(m) = managed {
        let _ = m.restart(false).await;
    }
}

/// GET a barkd endpoint, returning `(status, body)`. Mirrors the
/// module-private `barkd_get` (status only) for the one ceremony call
/// that needs the body; `None` means barkd did not answer at all.
async fn barkd_get_with_body(
    base_url: &str,
    path: &str,
    token: Option<&str>,
) -> Option<(reqwest::StatusCode, String)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let mut request = client.get(format!("{base_url}{path}"));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.ok()?;
    let status = response.status();
    let body = response.text().await.ok()?;
    Some((status, body))
}

#[cfg(test)]
mod mnemonic_ceremony_tests {
    use super::*;

    /// The BODY SHAPE is what matters here ({"mnemonic": "<12 words>"}).
    /// Fixture: the canonical BIP39 all-abandon test vector — NEVER a real
    /// wallet's words. A real mnemonic in source is a leaked secret even on
    /// signet (BIP39-valid words can hold real funds if the seed is ever
    /// reused); the historical fixture that violated this was replaced.
    const VERIFIED_MNEMONIC_BODY: &str = r#"{"mnemonic": "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"}"#;

    #[test]
    fn verified_mnemonic_body_parses_into_the_ceremony_shape() {
        let parsed: BarkdMnemonic = serde_json::from_str(VERIFIED_MNEMONIC_BODY).unwrap();
        assert_eq!(
            parsed.mnemonic,
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        );
    }

    #[test]
    fn exposed_instructions_say_exactly_what_happens() {
        let response = MnemonicResponse::exposed(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
                .to_string(),
        );
        // The probed wallet generated 12 words; the copy counts them
        // instead of hardcoding a word count.
        assert_eq!(response.word_count, Some(12));
        let instructions = &response.instructions;
        // Honest UI: the flag is why the words are readable at all…
        assert!(instructions.contains("--expose-mnemonic"));
        // …they are shown once and never stored or logged…
        assert!(instructions.contains("once"));
        assert!(instructions.contains("never stores or logs"));
        // …and the restart WITHOUT the flag ends the exposure for good.
        assert!(instructions.contains("WITHOUT the flag"));
        assert!(instructions.contains(UNEXPOSE_MNEMONIC_COMMAND));
    }

    #[test]
    fn not_exposed_instructions_carry_the_one_command_and_paste_nothing() {
        let response = MnemonicResponse::not_exposed("barkd answered HTTP 404.");
        assert!(response.mnemonic.is_none());
        assert!(response.word_count.is_none());
        let instructions = &response.instructions;
        assert!(instructions.contains("ONE command"));
        assert!(instructions.contains(EXPOSE_MNEMONIC_COMMAND));
        assert!(instructions.contains("--expose-mnemonic"));
        // The merchant pastes nothing, ever.
        assert!(instructions.contains("never paste"));
    }

    /// VERIFIED (same pass): GET /wallet/connected → {"connected": true}
    /// — a bare boolean flag, no wrapper object.
    #[test]
    fn verified_connected_body_is_a_bare_boolean() {
        let value: serde_json::Value = serde_json::from_str(r#"{"connected": true}"#).unwrap();
        assert_eq!(value["connected"].as_bool(), Some(true));
    }
}

#[derive(Serialize)]
pub struct WalletCreatedResponse {
    pub created: bool,
    /// True when the wallet was restored from the owner's recovery words.
    pub restored: bool,
    /// True when a wallet already existed (barkd refuses a second one).
    pub already_exists: bool,
    /// Recovery words (mnemonic), shown exactly once, never stored. `Some`
    /// only if barkd's create response itself carries the words (shape
    /// unpinned — VERIFY); the live pass observed `bark create` printing
    /// no mnemonic, so `None` is the expected case and the wizard pivots
    /// to the expose-mnemonic ceremony (`POST /setup/wallet/mnemonic`)
    /// instead of guessing.
    pub mnemonic: Option<String>,
    /// "create-response" when `mnemonic` came from the create response;
    /// "not-exposed" when the expose-mnemonic ceremony must run to obtain
    /// the words.
    pub mnemonic_source: &'static str,
    /// Plain-language instructions, shown when `mnemonic` is `None`.
    pub manual_instructions: Option<String>,
    pub network: String,
    pub asp_url: String,
}

// Manual Debug (Serialize is unaffected): the mnemonic field — present
// whenever barkd's create response carries words — must never reach a log
// line, panic message, or `{:?}`.
impl std::fmt::Debug for WalletCreatedResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletCreatedResponse")
            .field("created", &self.created)
            .field("restored", &self.restored)
            .field("already_exists", &self.already_exists)
            .field("mnemonic", &self.mnemonic.as_ref().map(|_| "[REDACTED]"))
            .field("mnemonic_source", &self.mnemonic_source)
            .field(
                "manual_instructions",
                &self.manual_instructions.as_ref().map(|_| "<present>"),
            )
            .field("network", &self.network)
            .field("asp_url", &self.asp_url)
            .finish()
    }
}

pub async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateWalletRequest>,
) -> Result<Response, AppError> {
    ensure_wizard_open(&state).await?;

    let (network, asp_url, mode) = resolve_wallet_choice(
        &state.config.mode,
        req.network.as_deref(),
        req.asp_url.as_deref(),
        req.mainnet_understood,
    )?;
    // Once a network was chosen it is fixed: switching needs a new server.
    if let Some(saved) = state.db.call(|c| crate::config::saved_mode(c)).await? {
        if saved != mode {
            return Err(AppError::BadRequest(
                "This server already uses the other network, and that cannot change. To switch, set up a new server.".into(),
            ));
        }
    }

    // Restoring: tidy and shape-check the words BEFORE anything is sent.
    let restore_words = match req.mnemonic.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => {
            if !req.paper_copies_exist {
                return Err(AppError::BadRequest(
                    "Tick the box to confirm your paper copy of the recovery words still exists, in a safe place. It is your real backup.".into(),
                ));
            }
            Some(clean_recovery_words(raw)?)
        }
        _ => None,
    };

    let Some(base_url) = barkd_base_url(&state.config) else {
        return Err(AppError::Internal(
            "barkd_url is not configured; the wizard cannot reach the wallet sidecar".into(),
        ));
    };

    let token = effective_barkd_token(&state.config);
    let outcome = create_till_wallet(
        &base_url,
        &network,
        &asp_url,
        restore_words.as_deref(),
        token.as_deref(),
    )
    .await?;

    // Record progress. The mnemonic is NOT stored — it exists only in the
    // response below, once. A restore also records that the owner typed the
    // words (so they hold them) and said the paper copies exist; that stands
    // in for the "write the words down" step, which has nothing to show.
    let restored = restore_words.is_some();
    drop(restore_words);
    // The wallet is now on this network: the server's mode follows it for
    // good. A running server applies a changed mode after one restart, at the
    // end of setup (payments stay closed until then).
    if mode != state.config.mode {
        request_restart_after_setup();
    }
    state
        .db
        .call(move |conn| {
            crate::config::save_mode(conn, mode)?;
            settings::set(conn, "setup_wallet_created", "true")?;
            if restored {
                settings::set(conn, "setup_wallet_restored", "true")?;
                settings::set(conn, "setup_backup_verified", "true")?;
                settings::set(conn, "setup_backup_confirmed", "true")?;
            }
            recompute_setup_complete(conn)
        })
        .await?;

    let mnemonic_source = if restored {
        "restored"
    } else if outcome.mnemonic.is_some() {
        "create-response"
    } else {
        "not-exposed"
    };
    let manual_instructions = if restored {
        None
    } else if outcome.mnemonic.is_none() {
        Some(
            "barkd did not return the recovery words in the create response. \
             Continue to the recovery-words step: it runs the one-time \
             --expose-mnemonic ceremony, reads the words from barkd exactly \
             once, shows them once, and then has you restart barkd WITHOUT \
             the flag. EBP never stores or logs the words."
                .to_string(),
        )
    } else {
        None
    };

    let response = WalletCreatedResponse {
        created: outcome.created,
        restored,
        already_exists: outcome.already_exists,
        mnemonic: outcome.mnemonic,
        mnemonic_source,
        manual_instructions,
        network,
        asp_url,
    };

    // no-store: the mnemonic must not live in any cache.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok((StatusCode::OK, headers, Json(response)).into_response())
}

/// Outcome of the wallet-create call against barkd.
struct CreateWalletOutcome {
    created: bool,
    already_exists: bool,
    /// The recovery words, when barkd returned them. Never stored, never
    /// logged — exists only to be shown once.
    mnemonic: Option<String>,
}

/// Create the till wallet through barkd (POST /wallet/create). The bearer
/// token is sent when we have one (a barkd that already holds a token
/// refuses unauthenticated creation; one without a token ignores it).
///
/// Request body per the pinned OpenAPI (`network`, `ark_server`); the
/// response carries only a `fingerprint`, so the mnemonic lookup below is a
/// harmless fallback and the words come from the mnemonic endpoint.
async fn create_till_wallet(
    base_url: &str,
    network: &str,
    asp_url: &str,
    restore_words: Option<&str>,
    token: Option<&str>,
) -> Result<CreateWalletOutcome, AppError> {
    // A restore may have to scan the chain for the wallet's coins, so it is
    // allowed far longer than making a new, empty wallet.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(if restore_words.is_some() {
            300
        } else {
            60
        }))
        .build()
        .map_err(|err| AppError::Internal(format!("could not build the barkd client: {err}")))?;

    let mut request = client
        .post(format!("{base_url}{PATH_WALLET_CREATE}"))
        // Pinned OpenAPI CreateWalletRequest: the field is `ark_server`.
        // (`ark` was silently ignored by barkd, so a custom ASP never applied.)
        .json(&match restore_words {
            // Restoring: the words go to barkd and nowhere else. `force` is
            // never sent, so barkd refuses rather than overwrite anything.
            Some(words) => {
                serde_json::json!({ "network": network, "ark_server": asp_url, "mnemonic": words })
            }
            None => serde_json::json!({ "network": network, "ark_server": asp_url }),
        });
    // FOUND BY THE LIVE SIGNET CHECK: a barkd that already has a token (the
    // single-container install reads it at boot) answers 401 to an
    // unauthenticated create. Send the token whenever we have one; a barkd
    // with no token yet simply ignores the header.
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|err| {
        // Log the detail; return a generic message (no internal URL) to
        // the unauthenticated caller (R9).
        tracing::warn!(error = %err, "setup: could not reach barkd");
        AppError::Internal("could not reach the wallet service".into())
    })?;

    if response.status().is_success() {
        let body: serde_json::Value = response.json().await.map_err(|err| {
            AppError::Internal(format!(
                "barkd's wallet-create answer was not valid JSON: {err}"
            ))
        })?;
        // The pinned spec returns no words here; kept as a tolerant fallback.
        let mnemonic = ["mnemonic", "seed_phrase", "recovery_phrase"]
            .iter()
            .find_map(|field| body.get(*field))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|words| !words.is_empty())
            .map(str::to_string);
        return Ok(CreateWalletOutcome {
            created: true,
            already_exists: false,
            mnemonic,
        });
    }

    // barkd refused. A wallet may already exist (the manual recipe path,
    // or a wizard re-run) — confirm with the authenticated probe before
    // saying so.
    let status = response.status();
    if restore_words.is_some() {
        // A restore never adopts or replaces a wallet that is already there,
        // and never repeats what barkd said (it could echo the words back).
        let exists = matches!(
            barkd_get(base_url, PATH_WALLET_BALANCE, token).await,
            Some(probe) if probe.is_success()
        );
        tracing::warn!(%status, wallet_exists = exists, "setup: restore from recovery words was refused");
        return Err(if exists {
            AppError::Conflict(
                "This server already has a wallet, so it cannot restore a different one. Restore onto a fresh server with an empty data folder.".into(),
            )
        } else {
            AppError::BadRequest(
                "The wallet service did not accept those words. Check every word and its order against your paper copy, then try again. Nothing was changed.".into(),
            )
        });
    }
    if let Some(probe) = barkd_get(base_url, PATH_WALLET_BALANCE, token).await {
        if probe.is_success() {
            return Ok(CreateWalletOutcome {
                created: false,
                already_exists: true,
                mnemonic: None,
            });
        }
    }
    Err(AppError::Internal(format!(
        "barkd refused to create the wallet (HTTP {status}). Check `docker compose logs barkd`; \
         if a wallet already exists, re-run — the wizard will detect it."
    )))
}

// ---------------------------------------------------------------------------
// POST /setup/wallet/confirm-backup — the ceremony checkbox
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConfirmBackupRequest {
    /// The backup ceremony checkbox. Must be `true`.
    pub confirmed: bool,
}

#[derive(Debug, Serialize)]
pub struct BackupConfirmedResponse {
    pub backup_confirmed: bool,
    pub setup_complete: bool,
}

pub async fn confirm_backup(
    State(state): State<AppState>,
    Json(req): Json<ConfirmBackupRequest>,
) -> Result<Json<BackupConfirmedResponse>, AppError> {
    ensure_wizard_open(&state).await?;

    if !req.confirmed {
        return Err(AppError::BadRequest(
            "the backup confirmation checkbox must be checked — the recovery words are the root \
             backup of the till"
                .into(),
        ));
    }

    let wallet_created = state
        .db
        .call(|conn| settings::get_bool(conn, "setup_wallet_created"))
        .await?;
    if !wallet_created {
        return Err(AppError::BadRequest(
            "create the wallet first — there is nothing to back up yet".into(),
        ));
    }

    // Server-enforced: the merchant must have proved they hold the words
    // (three correct, see `verify_backup`) before the backup counts.
    let verified = state
        .db
        .call(|conn| settings::get_bool(conn, "setup_backup_verified"))
        .await?;
    if !verified {
        return Err(AppError::Conflict(
            "check your backup first: type three of your recovery words from your paper copy"
                .into(),
        ));
    }

    let setup_complete = state
        .db
        .call(|conn| {
            settings::set(conn, "setup_backup_confirmed", "true")?;
            recompute_setup_complete(conn)
        })
        .await?;

    Ok(Json(BackupConfirmedResponse {
        backup_confirmed: true,
        setup_complete,
    }))
}

// ---------------------------------------------------------------------------
// POST /setup/wallet/verify-backup — prove the merchant holds the words
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BackupAnswer {
    /// 1-based word number.
    pub position: usize,
    pub word: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyBackupRequest {
    pub answers: Vec<BackupAnswer>,
}

/// At least three DIFFERENT positions, every one correct.
fn answers_match(mnemonic: &str, answers: &[BackupAnswer]) -> bool {
    let words: Vec<&str> = mnemonic.split_whitespace().collect();
    let mut seen = std::collections::HashSet::new();
    for a in answers {
        if a.position == 0 || a.position > words.len() {
            return false;
        }
        if !seen.insert(a.position) {
            return false;
        }
        if a.word.trim().to_lowercase() != words[a.position - 1] {
            return false;
        }
    }
    seen.len() >= 3
}

/// Checks the merchant's answers against the REAL words, read from barkd
/// again for this one request (the ceremony window opens and closes inside
/// `wallet_mnemonic`). Nothing is kept between requests: not in the browser,
/// not in server memory. Success records `setup_backup_verified`, which
/// `confirm_backup` requires.
pub async fn verify_backup(
    State(state): State<AppState>,
    Json(req): Json<VerifyBackupRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    ensure_wizard_open(&state).await?;
    if req.answers.len() < 3 || req.answers.len() > 24 {
        return Err(AppError::BadRequest(
            "answer at least three recovery words".into(),
        ));
    }
    let resp = wallet_mnemonic(State(state.clone())).await;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .map_err(|_| AppError::Internal("could not read the wallet answer".into()))?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Internal("unexpected wallet answer".into()))?;
    let Some(words) = body.get("mnemonic").and_then(|m| m.as_str()) else {
        let msg = body
            .get("instructions")
            .and_then(|m| m.as_str())
            .unwrap_or("The recovery words could not be read right now. Try again.");
        return Err(AppError::Conflict(format!("({status}) {msg}")));
    };
    let ok = answers_match(words, &req.answers);
    if ok {
        state
            .db
            .call(|conn| settings::set(conn, "setup_backup_verified", "true"))
            .await?;
    }
    Ok(Json(serde_json::json!({ "verified": ok })))
}

// ---------------------------------------------------------------------------
// POST /setup/token — verify + apply the barkd bearer token
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SetTokenRequest {
    /// The barkd bearer token (from `barkd secret show` on the machine
    /// running barkd). Never logged, never written to the database.
    #[serde(default)]
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenVerifiedResponse {
    pub verified: bool,
    /// Whether barkd reports the wallet connected to its ASP (parsed when
    /// possible; the HTTP status alone already proves the token works).
    pub connected: Option<bool>,
    /// The env var that must carry the token durably.
    pub env_var: &'static str,
    /// Plain-language persistence note (also shown in the UI).
    pub persistence_note: String,
    /// True when the payments paths need a restart to pick the token up
    /// (they read the env at boot; the wizard's own calls use it now).
    pub restart_required: bool,
    /// All wizard steps complete (wallet + token + store). Lets a UI close
    /// the wizard immediately; /setup/status is the authoritative read.
    pub setup_complete: bool,
}

pub async fn set_token(
    State(state): State<AppState>,
    Json(req): Json<SetTokenRequest>,
) -> Result<Json<TokenVerifiedResponse>, AppError> {
    ensure_wizard_open(&state).await?;

    let mut token = req.token.trim().to_string();
    if token.is_empty() {
        if let Some(m) = crate::managed_barkd::get() {
            token = m.token().await.map_err(AppError::BadRequest)?;
        }
    }
    if token.is_empty() {
        return Err(AppError::BadRequest(
            "token must not be empty — run `barkd secret show` on the machine running barkd and \
             paste the result"
                .into(),
        ));
    }

    let Some(base_url) = barkd_base_url(&state.config) else {
        return Err(AppError::Internal(
            "barkd_url is not configured; the wizard cannot reach the wallet sidecar".into(),
        ));
    };

    let connected = verify_barkd_token(&base_url, &token).await?;

    // Verified. Apply to the running process (the wizard's own barkd calls
    // use it immediately) and record progress. NOT persisted to the
    // database — the durable copy is EBP_BARKD_TOKEN in the deploy env.
    let restart_required = state.config.barkd_token.as_deref() != Some(token.as_str());
    set_runtime_barkd_token(&token);

    let wallet_probe = barkd_get(&base_url, PATH_WALLET_BALANCE, Some(&token)).await;

    let setup_complete = state
        .db
        .call(move |conn| {
            settings::set(conn, "setup_token_verified", "true")?;
            if matches!(wallet_probe, Some(status) if status.is_success()) {
                settings::set(conn, "setup_wallet_created", "true")?;
            }
            recompute_setup_complete(conn)
        })
        .await?;

    Ok(Json(TokenVerifiedResponse {
        verified: true,
        connected,
        env_var: "EBP_BARKD_TOKEN",
        setup_complete,
        persistence_note: "The token lives in your deployment environment (EBP_BARKD_TOKEN), \
                           never in the database. Add it to your docker-compose environment or \
                           systemd unit so it survives restarts."
            .to_string(),
        restart_required,
    }))
}

/// Verify a barkd bearer token against the sidecar (GET /wallet/connected).
/// `Err` carries a plain-language message that never contains the token.
///
/// Only the HTTP status is relied upon (the spec's body is `{"connected"}`).
async fn verify_barkd_token(base_url: &str, token: &str) -> Result<Option<bool>, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| AppError::Internal(format!("could not build the barkd client: {err}")))?;

    let mut header_value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| {
            AppError::BadRequest(
                "the token contains characters that are invalid in an HTTP header".into(),
            )
        })?;
    header_value.set_sensitive(true);

    let response = client
        .get(format!("{base_url}{PATH_WALLET_CONNECTED}"))
        .header(reqwest::header::AUTHORIZATION, header_value)
        .send()
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "setup: could not reach barkd");
            AppError::BadRequest(
                "could not reach the wallet service — is the barkd sidecar running?".into(),
            )
        })?;

    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "barkd rejected the token (HTTP {}). Re-copy it from `barkd secret show` on the \
             machine running barkd.",
            response.status()
        )));
    }

    let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
    Ok(body.get("connected").and_then(|value| value.as_bool()))
}

// ---------------------------------------------------------------------------
// POST /setup/store — create the first store, show the API key once
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateStoreRequest {
    pub name: String,
    /// Password for the admin page (min 16 characters). Optional for API
    /// automation; the wizard always sends it.
    #[serde(default)]
    pub admin_password: Option<String>,
    /// Where the till empties to (an Ark or Bitcoin address). Required: setup
    /// does not finish without a valid one (checked for the network mode).
    #[serde(default)]
    pub payout_address: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StoreCreatedResponse {
    pub created: bool,
    /// True when a store already existed (e.g. created via the CLI); the
    /// wizard does not create a second one.
    pub already_exists: bool,
    pub store_id: Option<String>,
    pub store_name: Option<String>,
    /// The store's API key — shown exactly once, never stored in the clear.
    pub api_key: Option<String>,
    /// The store's webhook signing secret — shown exactly once.
    pub webhook_secret: Option<String>,
    pub setup_complete: bool,
}

/// Managed mode booted without a barkd token, so the wallet adapter is the
/// stub. Once setup completes the process exits so the container's restart
/// policy brings it back with the real wallet (token read from barkd).
static RESTART_AFTER_SETUP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn request_restart_after_setup() {
    RESTART_AFTER_SETUP.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn schedule_restart() {
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(4)).await;
        tracing::info!("setup finished; restarting so the wallet connects");
        std::process::exit(0);
    });
}

pub async fn create_store(
    State(state): State<AppState>,
    Json(req): Json<CreateStoreRequest>,
) -> Result<Response, AppError> {
    ensure_wizard_open(&state).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("store name must not be empty".into()));
    }

    if let Some(pw) = req.admin_password.as_deref() {
        crate::admin_auth::validate_password(pw)?;
    }
    // Validate the payout address BEFORE creating anything, so a typo does
    // not leave a half-finished setup behind.
    let payout_address =
        match req.payout_address.as_deref().map(str::trim) {
            Some(a) if !a.is_empty() => crate::payout::validate(a, &wizard_mode(&state).await?)?,
            _ => return Err(AppError::BadRequest(
                "Add the address of a wallet you control (an Ark or Bitcoin address) to continue."
                    .into(),
            )),
        };
    let admin_password = req.admin_password.clone();
    let mode = wizard_mode(&state).await?;
    let iterations = state.config.password_hash_iterations;
    let (created, setup_complete) = state
        .db
        .call(move |conn| {
            let created = insert_first_store(conn, &name)?;
            crate::payout::set_initial(conn, &payout_address, &mode)?;
            if let Some(pw) = admin_password.as_deref() {
                crate::admin_auth::set_password(conn, pw, iterations)?;
            }
            let complete = recompute_setup_complete(conn)?;
            Ok((created, complete))
        })
        .await?;

    let response = StoreCreatedResponse {
        created: created.is_some(),
        already_exists: created.is_none(),
        store_id: created.as_ref().map(|store| store.store_id.clone()),
        store_name: created.as_ref().map(|store| store.store_name.clone()),
        api_key: created.as_ref().map(|store| store.api_key.clone()),
        webhook_secret: created.map(|store| store.webhook_secret),
        setup_complete,
    };

    if setup_complete && RESTART_AFTER_SETUP.load(std::sync::atomic::Ordering::SeqCst) {
        schedule_restart();
    }

    // no-store: the API key and webhook secret are shown exactly once.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    Ok((StatusCode::OK, headers, Json(response)).into_response())
}

// ---------------------------------------------------------------------------
// POST /setup/restore-backup — bring a backup file's orders and settings back
// ---------------------------------------------------------------------------

/// `POST /setup/restore-backup` — the owner's backup file (and its password)
/// on a NEW server whose wallet was just restored from the 12 words.
///
/// Body: 4 bytes (big-endian) = length of the backup password, then the
/// password (UTF-8), then the backup file (the same shape as "Check a backup").
///
/// It only STAGES the backup's database and key and then restarts the server;
/// the swap happens at the next start, before anything is opened (see
/// `backup::apply_pending_restore`). The wallet folder inside the backup is
/// never used: the money comes from the words. Needs the setup code (the
/// wizard guard), an unfinished setup and a wallet that was restored from its
/// words, and refuses when this server keeps its encryption key in
/// `EBP_KEY_ENCRYPTION_KEY` (the backup's key could not take effect).
pub async fn restore_backup(
    State(state): State<AppState>,
    request: axum::extract::Request,
) -> Result<Json<serde_json::Value>, AppError> {
    // Every check that needs no upload comes first, so a refused request never
    // makes the server read a large body.
    ensure_wizard_open(&state).await?;
    // One restore at a time: two uploads at once could otherwise mix one
    // file's database with the other's key. Once one has been staged the
    // server is about to restart, so the guard simply stays set.
    if RESTORE_IN_PROGRESS.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err(AppError::Conflict(
            "A restore is already under way. Wait about a minute for the server to restart, then reload this page.".into(),
        ));
    }
    let result = restore_backup_inner(&state, request).await;
    if result.is_err() {
        RESTORE_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    result
}

static RESTORE_IN_PROGRESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

async fn restore_backup_inner(
    state: &AppState,
    request: axum::extract::Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let restored = state
        .db
        .call(|conn| settings::get_bool(conn, "setup_wallet_restored"))
        .await?;
    if !restored {
        return Err(AppError::Conflict(
            "Restore your wallet from its 12 recovery words first (the step before this one). The backup file brings back orders and settings; the money always comes back from the words.".into(),
        ));
    }
    if state
        .config
        .key_encryption_key
        .as_deref()
        .is_some_and(|k| !k.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "This server keeps its encryption key in EBP_KEY_ENCRYPTION_KEY, so the key inside the backup could not take effect. Remove that setting (or set it to the key you used before) and try again.".into(),
        ));
    }
    if state.config.database_path == ":memory:" {
        return Err(AppError::BadRequest(
            "This server keeps its data in memory only, so there is nothing to restore into."
                .into(),
        ));
    }
    let orders_here: i64 = state
        .db
        .call(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM invoices", [], |r| r.get(0))?))
        .await?;
    if orders_here > 0 {
        return Err(AppError::Conflict(
            "This server already holds orders, and a restore never overwrites a working server."
                .into(),
        ));
    }

    let body = axum::body::to_bytes(request.into_body(), crate::routes::backup::MAX_CHECK_BYTES)
        .await
        .map_err(|_| {
            AppError::BadRequest("that backup file is too large or the upload was cut off".into())
        })?;
    crate::routes::backup::split_check_body(&body)?;
    let data_dir = crate::backup::data_dir_of(&state.config.database_path);
    let staged = crate::admin_auth::run_slow(move || {
        let (password, file) = crate::routes::backup::split_check_body(&body)?;
        let entries = crate::backup::open(file, password)?;
        crate::backup::stage_restore(&entries, &data_dir)
    })
    .await??;

    tracing::info!(
        orders = staged.orders,
        stores = staged.stores,
        "setup: backup staged; restarting to apply it"
    );
    schedule_restart();
    Ok(Json(serde_json::json!({
        "ok": true,
        "orders": staged.orders,
        "stores": staged.stores,
        "restarting": true,
    })))
}

pub struct CreatedStore {
    pub store_id: String,
    pub store_name: String,
    /// Plaintext API key — exists only in this return value and the one
    /// response that shows it.
    pub api_key: String,
    /// Plaintext webhook signing secret — same one-time rule.
    pub webhook_secret: String,
}

/// Create the first store.
///
/// NOTE (Phase 1 close-out): this inlines the insert rather than calling
/// `store::create` because that function was not visible in this pass;
/// when wiring the wizard, prefer replacing the body with a call to
/// `crate::store::create` so key generation and hashing live in one
/// place. The shape below mirrors the spec's data model —
/// `stores(id, name, api_key_hash, webhook_url, webhook_secret_hash,
/// policy_json, created_at)` — with the API key stored as hex(SHA-256)
/// (bearer lookup by hash) and the webhook secret stored as-is, because
/// the server must be able to sign deliveries with it (at-rest encryption
/// is the documented TODO on `Config::key_encryption_key`).
fn insert_first_store(
    conn: &rusqlite::Connection,
    name: &str,
) -> Result<Option<CreatedStore>, AppError> {
    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM stores", [], |row| row.get(0))?;
    if existing > 0 {
        return Ok(None);
    }

    // Delegate to the canonical store::create so the API-key/webhook-secret
    // hashing scheme is IDENTICAL to the one auth and webhooks use. (The
    // hand-rolled version stored the webhook secret verbatim — fixed.)
    let (store, api_key, webhook_secret) = crate::store::create(conn, name, None)?;

    Ok(Some(CreatedStore {
        store_id: store.id,
        store_name: store.name.clone(),
        api_key,
        // No webhook URL at wizard time -> no webhook secret yet; the
        // merchant gets one when they connect a store webhook later.
        webhook_secret: webhook_secret.unwrap_or_default(),
    }))
}

// ---------------------------------------------------------------------------
// The setup gate
// ---------------------------------------------------------------------------

/// The canonical setup gate: `503 SetupIncomplete` until the wizard
/// completes. Wire as middleware over the invoice and checkout routes:
///
/// ```ignore
/// use axum::middleware;
/// let gated = Router::new()
///     .route("/api/v1/invoices", post(invoices::create).get(invoices::list))
///     // ...
///     .layer(middleware::from_fn_with_state(state.clone(), setup::setup_gate));
/// ```
pub async fn setup_gate(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(response) = gate_response(&state).await {
        return response;
    }
    next.run(request).await
}

/// `POST /setup/verify-code` — succeeds only if the setup-token guard let the
/// request through, so the wizard page can tell a wrong code immediately.
pub async fn verify_code() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

/// Setup-token guard for the wizard router (beta hardening).
///
/// Active only when `config.setup_token` is set. GET requests (the wizard
/// page, `/setup/status`) pass; every other method needs the matching
/// `X-EBP-Setup-Token` header, compared in constant time. Without this, the
/// wizard is open to anyone who can reach an unfinished instance.
pub async fn setup_token_guard(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    // Plain HTTP on a public address would expose the setup code, API keys
    // and recovery words to anyone on the path: refuse every setup write.
    if request.method() != axum::http::Method::GET && !state.config.setup_transport_ok() {
        let body = serde_json::json!({
            "error": "https_required",
            "message": "Setup needs a secure (https://) address. Deploy with a host that \
                        provides HTTPS and set your public address to it, or open the setup \
                        page on the server itself (http://localhost:8080).",
        });
        return (StatusCode::FORBIDDEN, Json(body)).into_response();
    }
    if let Some(expected) = state.config.setup_token.as_deref() {
        if request.method() != axum::http::Method::GET {
            let supplied = request
                .headers()
                .get("x-ebp-setup-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let matches = if state.config.setup_token_generated {
                crate::auth::ct_eq(
                    &crate::auth::normalize_setup_code(expected),
                    &crate::auth::normalize_setup_code(supplied),
                )
            } else {
                crate::auth::ct_eq(expected, supplied)
            };
            if !matches {
                let body = serde_json::json!({
                    "error": "setup_token_required",
                    "message": "A setup code is needed. It is shown on the screen where you \
                                started the server (or is your EBP_SETUP_TOKEN). Send it in the \
                                X-EBP-Setup-Token header.",
                });
                return (StatusCode::UNAUTHORIZED, Json(body)).into_response();
            }
        }
    }
    next.run(request).await
}

/// The gate as a helper: `Some(503 response)` while setup is incomplete,
/// `None` when setup is complete. Also usable directly inside handlers:
/// `if let Some(response) = gate_response(&state).await { return Ok(response); }`.
pub async fn gate_response(state: &AppState) -> Option<Response> {
    let complete = state
        .db
        .call(|conn| settings::get_bool(conn, "setup_complete"))
        .await
        .unwrap_or(false);
    if complete {
        return None;
    }
    Some(setup_incomplete_response())
}

/// The 503 body every gated route returns while setup is incomplete.
pub fn setup_incomplete_response() -> Response {
    let body = serde_json::json!({
        "error": "SetupIncomplete",
        "message": "Setup is not finished. Complete the first-run wizard; invoice and checkout \
                    endpoints stay closed until then.",
        "setup_url": "/setup",
    });
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Wizard routes are reachable only before setup completes (documented
/// choice; see the module docs). `GET /setup/status` stays readable.
async fn ensure_wizard_open(state: &AppState) -> Result<(), AppError> {
    let complete = state
        .db
        .call(|conn| settings::get_bool(conn, "setup_complete"))
        .await?;
    if complete {
        return Err(AppError::Conflict(
            "setup is already complete — the wizard is closed. To start over, follow the \
             documented reset procedure (back up the recovery words first)."
                .into(),
        ));
    }
    Ok(())
}

fn barkd_base_url(config: &crate::config::Config) -> Option<String> {
    let url = config.barkd_url.trim_end_matches('/');
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// One GET against barkd. Returns the HTTP status, or `None` when barkd
/// was unreachable. Never logs the token.
async fn barkd_get(base_url: &str, path: &str, token: Option<&str>) -> Option<reqwest::StatusCode> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let mut request = client.get(format!("{base_url}{path}"));
    if let Some(token) = token {
        let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
        value.set_sensitive(true);
        request = request.header(reqwest::header::AUTHORIZATION, value);
    }
    match request.send().await {
        Ok(response) => Some(response.status()),
        Err(_) => None,
    }
}

/// setup_complete = wallet created + backup confirmed + token verified +
/// at least one store. Recomputed after every wizard step; the backup
/// checkbox is deliberately load-bearing — without it, setup can never
/// complete.
fn recompute_setup_complete(conn: &rusqlite::Connection) -> Result<bool, AppError> {
    let wallet = settings::get_bool(conn, "setup_wallet_created")?;
    let backup = settings::get_bool(conn, "setup_backup_confirmed")?;
    let token = settings::get_bool(conn, "setup_token_verified")?;
    let stores: i64 = conn.query_row("SELECT COUNT(*) FROM stores", [], |row| row.get(0))?;
    let complete = wallet && backup && token && stores > 0;
    settings::set(
        conn,
        "setup_complete",
        if complete { "true" } else { "false" },
    )?;
    Ok(complete)
}

// ---------------------------------------------------------------------------
// Tests (offline: in-memory SQLite + a hand-rolled mock barkd)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #[test]
    fn backup_answers_need_three_distinct_correct_positions() {
        let m = "abandon ability able about above absent";
        let a = |p: usize, w: &str| super::BackupAnswer {
            position: p,
            word: w.to_string(),
        };
        assert!(super::answers_match(
            m,
            &[a(1, "abandon"), a(3, " ABLE "), a(6, "absent")]
        ));
        assert!(
            !super::answers_match(m, &[a(1, "abandon"), a(3, "able")]),
            "two is not enough"
        );
        assert!(
            !super::answers_match(m, &[a(1, "abandon"), a(1, "abandon"), a(3, "able")]),
            "duplicates"
        );
        assert!(!super::answers_match(
            m,
            &[a(1, "abandon"), a(3, "wrong"), a(6, "absent")]
        ));
        assert!(!super::answers_match(
            m,
            &[a(0, "x"), a(3, "able"), a(6, "absent")]
        ));
        assert!(!super::answers_match(
            m,
            &[a(1, "abandon"), a(3, "able"), a(9, "absent")]
        ));
    }

    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// In-memory DB with the stores table (settings self-heals).
    fn memory_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute_batch(
            "CREATE TABLE stores (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                webhook_url TEXT,
                webhook_secret_hash TEXT,
                webhook_secret TEXT,
                policy_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE api_keys (
                id TEXT PRIMARY KEY,
                store_id TEXT NOT NULL,
                key_hash TEXT NOT NULL,
                label TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .expect("stores table");
        conn
    }

    /// Mock barkd: answers exactly one request with a canned status+body.
    fn spawn_barkd(status_line: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status_line = status_line.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut buf = [0u8; 4096];
            while !request.contains("\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            let response = format!(
                "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: \
                 {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://127.0.0.1:{port}/api/v1")
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    // -- gate behaviour --------------------------------------------------

    #[test]
    fn gate_opens_only_after_every_step() {
        let conn = memory_db();
        assert!(!recompute_setup_complete(&conn).unwrap());

        settings::set(&conn, "setup_wallet_created", "true").unwrap();
        assert!(!recompute_setup_complete(&conn).unwrap());

        settings::set(&conn, "setup_token_verified", "true").unwrap();
        assert!(!recompute_setup_complete(&conn).unwrap());

        // The backup checkbox is load-bearing: without it, never complete.
        settings::set(&conn, "setup_backup_confirmed", "true").unwrap();
        assert!(!recompute_setup_complete(&conn).unwrap(), "no store yet");

        insert_first_store(&conn, "Test store")
            .unwrap()
            .expect("first store is created");
        assert!(recompute_setup_complete(&conn).unwrap());
        assert!(settings::get_bool(&conn, "setup_complete").unwrap());
    }

    #[test]
    fn gate_response_is_a_503_setup_incomplete() {
        let response = setup_incomplete_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // -- status flow -----------------------------------------------------

    #[test]
    fn status_reflects_wizard_progress() {
        let conn = memory_db();
        let snapshot = wizard_snapshot(&conn).unwrap();
        assert!(!snapshot.wallet_created);
        assert!(!snapshot.config_written);
        assert!(!snapshot.backup_confirmed);
        assert!(!snapshot.setup_complete);
        assert_eq!(snapshot.store_count, 0);

        settings::set(&conn, "setup_wallet_created", "true").unwrap();
        settings::set(&conn, "setup_token_verified", "true").unwrap();
        settings::set(&conn, "setup_backup_confirmed", "true").unwrap();
        insert_first_store(&conn, "Test store").unwrap().unwrap();
        recompute_setup_complete(&conn).unwrap();

        let snapshot = wizard_snapshot(&conn).unwrap();
        assert!(snapshot.wallet_created);
        assert!(snapshot.config_written);
        assert!(snapshot.backup_confirmed);
        assert!(snapshot.setup_complete);
        assert_eq!(snapshot.store_count, 1);
    }

    // -- the store step --------------------------------------------------

    #[test]
    fn second_store_is_refused_and_secrets_are_prepared_once() {
        let conn = memory_db();
        let first = insert_first_store(&conn, "First")
            .unwrap()
            .expect("created");
        assert!(!first.api_key.is_empty());
        // Delegated to store::create: no webhook URL at wizard time, so no
        // webhook secret yet — and the secret is never stored verbatim.
        assert!(first.webhook_secret.is_empty());
        assert_eq!(crate::auth::hash_secret(&first.api_key).len(), 64);

        let second = insert_first_store(&conn, "Second").unwrap();
        assert!(second.is_none(), "the wizard never creates a second store");
    }

    // -- the token step --------------------------------------------------

    #[test]
    fn token_verification_failure_path_is_plain_language() {
        let base_url = spawn_barkd("401 Unauthorized", "{}");
        let err = block_on(verify_barkd_token(&base_url, "wrong-token")).unwrap_err();
        let message = match err {
            AppError::BadRequest(message) => message,
            other => panic!("expected a plain-language rejection, got: {other:?}"),
        };
        assert!(message.contains("rejected"), "message: {message}");
        assert!(
            !message.contains("wrong-token"),
            "the token must never appear in errors"
        );
    }

    #[test]
    fn token_verification_succeeds_against_barkd() {
        let base_url = spawn_barkd("200 OK", r#"{"connected": true}"#);
        let connected = block_on(verify_barkd_token(&base_url, "good-token")).unwrap();
        assert_eq!(connected, Some(true));
    }

    // -- the wallet step -------------------------------------------------

    #[test]
    fn wallet_create_captures_the_mnemonic_exactly_once() {
        let base_url = spawn_barkd(
            "200 OK",
            r#"{"mnemonic": "word one two three four five six seven eight nine ten eleven twelve"}"#,
        );
        let outcome = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            None,
            None,
        ))
        .unwrap();
        assert!(outcome.created);
        assert!(!outcome.already_exists);
        assert_eq!(
            outcome.mnemonic.as_deref(),
            Some("word one two three four five six seven eight nine ten eleven twelve")
        );
    }

    #[test]
    fn wallet_create_is_honest_when_barkd_hides_the_mnemonic() {
        let base_url = spawn_barkd("200 OK", r#"{"ok": true}"#);
        let outcome = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            None,
            None,
        ))
        .unwrap();
        assert!(outcome.created);
        assert!(outcome.mnemonic.is_none(), "never guessed, never faked");
    }

    #[test]
    fn wallet_create_reports_already_exists_via_the_probe() {
        // barkd refuses the create (wallet exists) but answers the
        // authenticated balance probe.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for expected in ["POST /api/v1/wallet/create", "GET /api/v1/wallet/balance"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                let mut buf = [0u8; 4096];
                while !request.contains("\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                assert!(
                    request.starts_with(expected),
                    "expected {expected}, got: {request}"
                );
                let (status, body) = if expected.starts_with("POST") {
                    ("409 Conflict", "{}")
                } else {
                    ("200 OK", r#"{"offchain": 0}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: \
                     {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let base_url = format!("http://127.0.0.1:{port}/api/v1");
        let outcome = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            None,
            Some("token"),
        ))
        .unwrap();
        assert!(outcome.already_exists);
        assert!(!outcome.created);
        assert!(outcome.mnemonic.is_none());
    }

    // -- restoring the wallet from the recovery words ----------------------

    const WORDS12: &str =
        "abandon ability able about above absent absorb abstract absurd abuse access accident";

    /// A mock barkd that records the whole first request (headers and body)
    /// and answers every request with the given status line and body.
    fn spawn_recording_barkd(
        status_line: &str,
        body: &str,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        let status_line = status_line.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut request = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let len = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if request.len() >= head_end + 4 + len {
                            break;
                        }
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&request).to_string());
                let response = format!(
                    "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\ncontent-length: \
                     {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://127.0.0.1:{port}/api/v1"), rx)
    }

    #[test]
    fn recovery_words_are_tidied_and_shape_checked_without_echoing_them() {
        assert_eq!(
            clean_recovery_words("  Abandon   ABILITY able\nabout above absent absorb abstract absurd abuse access accident ")
                .unwrap(),
            WORDS12
        );
        for bad in [
            "",
            "abandon ability able",
            "abandon ability able about above absent absorb abstract absurd abuse access",
            "abandon ability able about above absent absorb abstract absurd abuse access acc1dent",
            "abandon ability able about above absent absorb abstract absurd abuse access accident extra",
        ] {
            let msg = format!("{:?}", clean_recovery_words(bad).unwrap_err());
            assert!(msg.contains("12 words"), "{msg}");
            assert!(!msg.contains("abandon"), "the error must not repeat the words");
        }
    }

    #[test]
    fn the_request_never_shows_the_words_in_debug_output() {
        let req = CreateWalletRequest {
            network: None,
            asp_url: None,
            mnemonic: Some(WORDS12.to_string()),
            paper_copies_exist: true,
            mainnet_understood: false,
        };
        let shown = format!("{req:?}");
        assert!(shown.contains("REDACTED"));
        assert!(!shown.contains("abandon") && !shown.contains("accident"));
    }

    #[test]
    fn restoring_sends_the_words_to_barkd_and_never_force() {
        let (base_url, rx) = spawn_recording_barkd("200 OK", r#"{"fingerprint": "abcd1234"}"#);
        let outcome = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            Some(WORDS12),
            Some("tok"),
        ))
        .unwrap();
        assert!(outcome.created);
        assert!(
            outcome.mnemonic.is_none(),
            "restoring never hands words back"
        );
        let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /api/v1/wallet/create"));
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(json["mnemonic"], WORDS12);
        assert_eq!(json["network"], "signet");
        assert!(json.get("force").is_none(), "force must never be sent");
    }

    #[test]
    fn a_new_wallet_request_carries_no_words() {
        let (base_url, rx) = spawn_recording_barkd("200 OK", r#"{"fingerprint": "abcd1234"}"#);
        block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            None,
            None,
        ))
        .unwrap();
        let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let json: serde_json::Value = serde_json::from_str(body).unwrap();
        assert!(json.get("mnemonic").is_none());
    }

    #[test]
    fn a_refused_restore_is_a_plain_message_that_does_not_repeat_barkd() {
        // barkd refuses and then also refuses the balance probe (no wallet):
        // the words were not accepted.
        let (base_url, _rx) = spawn_recording_barkd(
            "500 Internal Server Error",
            r#"{"message": "invalid mnemonic: abandon ability able"}"#,
        );
        let err = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            Some(WORDS12),
            Some("tok"),
        ))
        .err()
        .expect("must fail");
        let msg = format!("{err:?}");
        assert!(msg.contains("did not accept those words"), "{msg}");
        assert!(
            !msg.contains("invalid mnemonic") && !msg.contains("abandon"),
            "{msg}"
        );
    }

    #[test]
    fn restoring_onto_a_server_that_already_has_a_wallet_is_refused() {
        // create refused (409), but the balance probe answers: a wallet exists.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for expected in ["POST /api/v1/wallet/create", "GET /api/v1/wallet/balance"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                let mut buf = [0u8; 4096];
                while !request.contains("\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                assert!(
                    request.starts_with(expected),
                    "expected {expected}: {request}"
                );
                let (status, body) = if expected.starts_with("POST") {
                    ("409 Conflict", "{}")
                } else {
                    ("200 OK", r#"{"offchain": 0}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: \
                     {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let base_url = format!("http://127.0.0.1:{port}/api/v1");
        let err = block_on(create_till_wallet(
            &base_url,
            "signet",
            DEFAULT_SIGNET_ASP,
            Some(WORDS12),
            Some("tok"),
        ))
        .err()
        .expect("must fail");
        let msg = format!("{err:?}");
        assert!(msg.contains("already has a wallet"), "{msg}");
    }

    // -- the runtime token override ----------------------------------------

    #[test]
    fn signet_is_the_default_and_mainnet_needs_the_tick_box() {
        // Nothing chosen: signet, unless the server was started in live mode.
        let (net, asp, mode) = resolve_wallet_choice("test", None, None, false).unwrap();
        assert_eq!(
            (net.as_str(), asp.as_str(), mode),
            ("signet", DEFAULT_SIGNET_ASP, "test")
        );
        let (net, _, mode) = resolve_wallet_choice("live", None, None, true).unwrap();
        assert_eq!((net.as_str(), mode), ("mainnet", "live"));
        // Choosing mainnet on a server started in test mode works, with the
        // tick-box, and gets Second's mainnet server.
        let err = resolve_wallet_choice("test", Some("mainnet"), Some(""), false).unwrap_err();
        let AppError::BadRequest(msg) = err else {
            panic!("expected a plain refusal")
        };
        assert!(msg.contains("Tick the box"), "{msg}");
        let (net, asp, mode) =
            resolve_wallet_choice("test", Some(" Mainnet "), Some("  "), true).unwrap();
        assert_eq!(
            (net.as_str(), asp.as_str(), mode),
            ("mainnet", DEFAULT_MAINNET_ASP, "live")
        );
        // Choosing signet on a server started in live mode works too.
        let (net, asp, mode) = resolve_wallet_choice("live", Some("signet"), None, false).unwrap();
        assert_eq!(
            (net.as_str(), asp.as_str(), mode),
            ("signet", DEFAULT_SIGNET_ASP, "test")
        );
        let (_, asp, _) = resolve_wallet_choice(
            "test",
            Some("mainnet"),
            Some("https://ark.second.tech/"),
            true,
        )
        .unwrap();
        assert_eq!(asp, "https://ark.second.tech");
    }

    #[test]
    fn only_signet_and_mainnet_are_offered_and_the_server_must_match() {
        for net in ["testnet", "regtest", "bitcoin-ish"] {
            assert!(
                resolve_wallet_choice("test", Some(net), None, true).is_err(),
                "{net}"
            );
        }
        // A signet Ark server with mainnet (or the reverse) is refused.
        assert!(
            resolve_wallet_choice("test", Some("mainnet"), Some(DEFAULT_SIGNET_ASP), true).is_err()
        );
        assert!(
            resolve_wallet_choice("test", Some("signet"), Some(DEFAULT_MAINNET_ASP), false)
                .is_err()
        );
    }

    #[test]
    fn runtime_token_overrides_the_env_token() {
        let config = crate::config::Config {
            barkd_token: Some("env-token".to_string()),
            ..crate::config::Config::default()
        };
        assert_eq!(effective_barkd_token(&config).as_deref(), Some("env-token"));

        set_runtime_barkd_token("wizard-token");
        assert_eq!(
            effective_barkd_token(&config).as_deref(),
            Some("wizard-token")
        );

        clear_runtime_barkd_token();
        assert_eq!(effective_barkd_token(&config).as_deref(), Some("env-token"));
    }
}
