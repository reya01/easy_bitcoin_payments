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
//!   The wallet mnemonic, when barkd returns one at creation, is shown
//!   exactly once in the response body (with `Cache-Control: no-store`)
//!   and never stored or logged. No code path here logs either value.
//! * **The gate.** [`setup_gate`] is the canonical check that returns
//!   `503 SetupIncomplete` from invoice and checkout routes until setup
//!   completes; wire it as middleware over those routes (see the Phase 1
//!   close-out notes for the exact lines). [`gate_response`] is the same
//!   check as a helper for in-route use.
//! * **Honesty about barkd.** Wallet creation, token verification, and
//!   the wallet probe call the barkd REST API directly. Paths are marked
//!   `VERIFY` where the pinned OpenAPI does not pin them; response shapes
//!   are parsed defensively, and a missing mnemonic is reported honestly
//!   with manual instructions rather than guessed at.

use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::routes::AppState;
use crate::settings;
use crate::util::now_unix;

// ---------------------------------------------------------------------------
// barkd paths used by the wizard. Duplicated from src/wallet/barkd.rs
// (where the equivalents are module-private); scripts/check_barkd_drift.sh
// verifies every PATH_* constant in src/ — both copies — against the
// pinned OpenAPI. VERIFY markers per the honesty rule.
// ---------------------------------------------------------------------------
/// Create (or restore) the wallet. POST. No bearer token: wallet creation
/// precedes token provisioning (the verified recipe call sends none).
const PATH_WALLET_CREATE: &str = "/wallet/create";
/// Whether the wallet is connected to its ASP. GET.
const PATH_WALLET_CONNECTED: &str = "/wallet/connected";
/// On-chain and off-chain balances. GET. Used as a shape-agnostic
/// "does a wallet exist" probe (any 2xx = yes).
const PATH_WALLET_BALANCE: &str = "/wallet/balance";

/// Default ASP in test mode: Second's public signet Ark server.
const DEFAULT_SIGNET_ASP: &str = "https://ark.signet.2nd.dev";

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
    if let Some(token) = runtime_token_cell().read().ok().and_then(|cell| cell.clone()) {
        return Some(token);
    }
    config.barkd_token.clone()
}

/// Apply a wizard-verified token to the running process. The wizard's own
/// barkd calls use it immediately; the payments paths pick the token up
/// from the environment after the documented restart. Not persisted.
fn set_runtime_barkd_token(token: &str) {
    if let Ok(mut cell) = runtime_token_cell().write() {
        *cell = Some(token.to_string());
    }
}

/// Clear the runtime override (tests; also the honest "undo" if needed).
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
    /// True only after: wallet created + backup confirmed + token verified
    /// + at least one store. Invoice and checkout routes stay closed
    /// (503 SetupIncomplete) until then.
    pub setup_complete: bool,
    /// `"test"` (signet, play money) or `"live"`.
    pub mode: String,
}

pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, AppError> {
    let token = effective_barkd_token(&state.config);

    let mut snapshot = state.db.call(|conn| wizard_snapshot(conn)).await?;

    // Best-effort live probe (read-only): with a token we can ask barkd
    // directly whether a wallet exists. The flag itself is only persisted
    // by the wallet/token steps.
    if !snapshot.wallet_created && token.is_some() {
        if let Some(base_url) = barkd_base_url(&state.config) {
            if let Some(status) = barkd_get(&base_url, PATH_WALLET_BALANCE, token.as_deref()).await {
                if status.is_success() {
                    snapshot.wallet_created = true;
                }
            }
        }
    }

    snapshot.barkd_token_present = token.is_some();
    snapshot.mode = state.config.mode.clone();

    Ok(Json(snapshot))
}

/// The settings-table part of the status (everything except the two
/// runtime-filled fields). Separated so it is unit-testable.
fn wizard_snapshot(conn: &rusqlite::Connection) -> Result<SetupStatus, AppError> {
    Ok(SetupStatus {
        wallet_created: settings::get_bool(conn, "setup_wallet_created")?,
        barkd_token_present: false, // filled by the caller (runtime token)
        config_written: settings::get_bool(conn, "setup_token_verified")?,
        backup_confirmed: settings::get_bool(conn, "setup_backup_confirmed")?,
        store_count: conn.query_row("SELECT COUNT(*) FROM stores", [], |row| row.get(0))?,
        setup_complete: settings::get_bool(conn, "setup_complete")?,
        mode: String::new(), // filled by the caller
    })
}

// ---------------------------------------------------------------------------
// POST /setup/wallet — create the till wallet, show the mnemonic once
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateWalletRequest {
    /// Bitcoin network. Defaults per `mode`: "signet" in test, "mainnet" in
    /// live.
    pub network: Option<String>,
    /// Ark Service Provider URL. Defaults to Second's public signet ASP in
    /// test mode; required in live mode.
    pub asp_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WalletCreatedResponse {
    pub created: bool,
    /// True when a wallet already existed (barkd refuses a second one).
    pub already_exists: bool,
    /// Recovery words (mnemonic), shown exactly once, never stored. `None`
    /// when barkd did not return them — see `manual_instructions`.
    pub mnemonic: Option<String>,
    /// "create-response" or "not-exposed".
    pub mnemonic_source: &'static str,
    /// Plain-language instructions, shown when `mnemonic` is `None`.
    pub manual_instructions: Option<String>,
    pub network: String,
    pub asp_url: String,
}

pub async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<CreateWalletRequest>,
) -> Result<Response, AppError> {
    ensure_wizard_open(&state).await?;

    let network = match req.network.as_deref() {
        Some(network) => network.trim().to_lowercase(),
        None if state.config.mode == "live" => "mainnet".to_string(),
        None => "signet".to_string(),
    };

    let asp_url = match req.asp_url.as_deref() {
        Some(url) => url.trim().trim_end_matches('/').to_string(),
        None if state.config.mode == "live" => {
            return Err(AppError::BadRequest(
                "asp_url is required in live mode — the address of the Ark server this wallet will use".into(),
            ));
        }
        None => DEFAULT_SIGNET_ASP.to_string(),
    };

    let Some(base_url) = barkd_base_url(&state.config) else {
        return Err(AppError::Internal(
            "barkd_url is not configured; the wizard cannot reach the wallet sidecar".into(),
        ));
    };

    let token = effective_barkd_token(&state.config);
    let outcome = create_till_wallet(&base_url, &network, &asp_url, token.as_deref()).await?;

    // Record progress. The mnemonic is NOT stored — it exists only in the
    // response below, once.
    state
        .db
        .call(|conn| {
            settings::set(conn, "setup_wallet_created", "true")?;
            recompute_setup_complete(conn)
        })
        .await?;

    let mnemonic_source = if outcome.mnemonic.is_some() {
        "create-response"
    } else {
        "not-exposed"
    };
    let manual_instructions = if outcome.mnemonic.is_none() {
        Some(
            "barkd did not return the recovery words, and EBP will never call the \
             mnemonic endpoint (--expose-mnemonic stays off). To see them, run \
             barkd's own wallet-export command on the machine running barkd \
             (see barkd's documentation), write them down on paper, and keep \
             them offline."
                .to_string(),
        )
    } else {
        None
    };

    let response = WalletCreatedResponse {
        created: outcome.created,
        already_exists: outcome.already_exists,
        mnemonic: outcome.mnemonic,
        mnemonic_source,
        manual_instructions,
        network,
        asp_url,
    };

    // no-store: the mnemonic must not live in any cache.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
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

/// Create the till wallet through barkd (POST /wallet/create, no bearer
/// token — creation precedes token provisioning; the verified recipe call
/// sends none either).
///
/// VERIFY: request body shape (`{"network", "ark"}`) and the response's
/// mnemonic field name — the pinned OpenAPI's schemas are empty.
async fn create_till_wallet(
    base_url: &str,
    network: &str,
    asp_url: &str,
    token: Option<&str>,
) -> Result<CreateWalletOutcome, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|err| AppError::Internal(format!("could not build the barkd client: {err}")))?;

    let response = client
        .post(format!("{base_url}{PATH_WALLET_CREATE}"))
        .json(&serde_json::json!({ "network": network, "ark": asp_url }))
        .send()
        .await
        .map_err(|err| AppError::Internal(format!("could not reach barkd at {base_url}: {err}")))?;

    if response.status().is_success() {
        let body: serde_json::Value = response.json().await.map_err(|err| {
            AppError::Internal(format!("barkd's wallet-create answer was not valid JSON: {err}"))
        })?;
        // VERIFY: field name — the recipe says the answer includes the
        // recovery words; the field name is not pinned by the spec.
        let mnemonic = ["mnemonic", "seed_phrase", "recovery_phrase"]
            .iter()
            .find_map(|field| body.get(*field))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|words| !words.is_empty())
            .map(str::to_string);
        return Ok(CreateWalletOutcome { created: true, already_exists: false, mnemonic });
    }

    // barkd refused. A wallet may already exist (the manual recipe path,
    // or a wizard re-run) — confirm with the authenticated probe before
    // saying so.
    let status = response.status();
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
// POST /setup/token — verify + apply the barkd bearer token
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SetTokenRequest {
    /// The barkd bearer token (from `barkd secret show` on the machine
    /// running barkd). Never logged, never written to the database.
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
}

pub async fn set_token(
    State(state): State<AppState>,
    Json(req): Json<SetTokenRequest>,
) -> Result<Json<TokenVerifiedResponse>, AppError> {
    ensure_wizard_open(&state).await?;

    let token = req.token.trim().to_string();
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
/// VERIFY: response shape — the pinned OpenAPI's schemas are empty; only
/// the HTTP status is relied upon.
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
            AppError::BadRequest(format!(
                "could not reach barkd at {base_url}: {err} — is the sidecar running?"
            ))
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

pub async fn create_store(
    State(state): State<AppState>,
    Json(req): Json<CreateStoreRequest>,
) -> Result<Response, AppError> {
    ensure_wizard_open(&state).await?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("store name must not be empty".into()));
    }

    let (created, setup_complete) = state
        .db
        .call(move |conn| {
            let created = insert_first_store(conn, &name)?;
            let complete = recompute_setup_complete(conn)?;
            Ok((created, complete))
        })
        .await?;

    let response = StoreCreatedResponse {
        created: created.is_some(),
        already_exists: created.is_none(),
        store_id: created.as_ref().map(|store| store.store_id.clone()),
        store_name: created.as_ref().map(|store| store.store_name.clone()),
        api_key: created.map(|store| store.api_key),
        webhook_secret: created.map(|store| store.webhook_secret),
        setup_complete,
    };

    // no-store: the API key and webhook secret are shown exactly once.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, headers, Json(response)).into_response())
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

    let store_id = crate::util::new_id("store");
    let api_key = crate::util::new_id("ebp");
    let webhook_secret = crate::util::new_id("whsec");
    let api_key_hash = hex_sha256(&api_key);

    conn.execute(
        "INSERT INTO stores (id, name, api_key_hash, webhook_url, webhook_secret_hash, \
         policy_json, created_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6)",
        rusqlite::params![
            store_id,
            name,
            api_key_hash,
            webhook_secret,
            "{}",
            now_unix(),
        ],
    )?;

    Ok(Some(CreatedStore {
        store_id,
        store_name: name.to_string(),
        api_key,
        webhook_secret,
    }))
}

/// Hex(SHA-256) of the API key — the at-rest form the bearer lookup uses.
/// NOTE: must match src/auth.rs's scheme; see the NOTE on
/// [`insert_first_store`].
fn hex_sha256(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
    settings::set(conn, "setup_complete", if complete { "true" } else { "false" })?;
    Ok(complete)
}

// ---------------------------------------------------------------------------
// Tests (offline: in-memory SQLite + a hand-rolled mock barkd)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// In-memory DB with the stores table (settings self-heals).
    fn memory_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory db");
        conn.execute(
            "CREATE TABLE stores (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                api_key_hash TEXT NOT NULL,
                webhook_url TEXT,
                webhook_secret_hash TEXT,
                policy_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
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
        let first = insert_first_store(&conn, "First").unwrap().expect("created");
        assert!(!first.api_key.is_empty());
        assert!(!first.webhook_secret.is_empty());
        assert_eq!(hex_sha256(&first.api_key).len(), 64);

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
        let outcome =
            block_on(create_till_wallet(&base_url, "signet", DEFAULT_SIGNET_ASP, None)).unwrap();
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
        let outcome =
            block_on(create_till_wallet(&base_url, "signet", DEFAULT_SIGNET_ASP, None)).unwrap();
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
            Some("token"),
        ))
        .unwrap();
        assert!(outcome.already_exists);
        assert!(!outcome.created);
        assert!(outcome.mnemonic.is_none());
    }

    // -- the runtime token override ----------------------------------------

    #[test]
    fn runtime_token_overrides_the_env_token() {
        let mut config = crate::config::Config::default();
        config.barkd_token = Some("env-token".to_string());
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