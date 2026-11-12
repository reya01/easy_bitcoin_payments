use std::sync::Arc;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::rates::RateService;
use easy_bitcoin_payments::routes::{router, AppState};
use easy_bitcoin_payments::store;
use easy_bitcoin_payments::wallet::StubWallet;
use easy_bitcoin_payments::watcher::PollingWatcher;
use easy_bitcoin_payments::webhook::WebhookEmitter;

#[tokio::main]
async fn main() {
    // Print a problem in plain words (not Rust's "Error: Invalid { key: ... }"
    // debug form) and exit with a failure code.
    if let Err(e) = run().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");

    match command {
        "serve" => serve().await,
        "store" => store_command(&args[1..]).await,
        "admin" => admin_command(&args[1..]).await,
        "backup" => backup_command(&args[1..]),
        "help" | "--help" | "-h" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_help();
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "ebp {version} — Easy Bitcoin Payments\n\
         \n\
         USAGE:\n\
         \x20 ebp serve                       Start the payments server\n\
         \x20 ebp store create --name <NAME>  Create a store and print its API key once\n\
         \x20     [--webhook-url <URL>]\n\
         \x20 ebp store list                  List stores (id and name)\n\
         \x20 ebp store rotate-key [--store <ID>]\n\
         \x20     Revoke every API key of a store and print a new one once\n\
         \x20     (lost-key recovery; --store is optional with one store)\n\
         \x20 ebp admin set-password          Set/reset the admin-page password\n\
         \x20 ebp admin reset-2fa             Turn off two-step sign-in (lost phone)\n\
         \x20 ebp backup verify <FILE>       Check a downloaded backup opens (asks for its password)\n\
         \x20 ebp backup restore <FILE> <NEW-FOLDER>\n\
         \x20     Unpack a backup into a new, empty folder (never overwrites)\n\
         \x20 ebp help                        Show this help\n\
         \n\
         CONFIGURATION:\n\
         \x20 Environment variables (EBP_*) and an optional TOML file at ./ebp.toml\n\
         \x20 (or the path in EBP_CONFIG). See ebp.example.toml.\n",
        version = easy_bitcoin_payments::VERSION
    );
}

async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // A backup staged by the setup wizard is put in place now, before the key
    // or the database is opened. Nothing is deleted; a refusal only logs.
    match easy_bitcoin_payments::backup::apply_pending_restore(&config.database_path) {
        Ok(Some(done)) => tracing::warn!("{done}"),
        Ok(None) => {}
        Err(why) => {
            tracing::error!("{why}");
            easy_bitcoin_payments::backup::note_restore_refused(why);
        }
    }

    // R10: webhook secrets are encrypted at rest. A malformed configured key
    // aborts startup rather than silently falling back to another key.
    easy_bitcoin_payments::secrets::init(
        config.key_encryption_key.as_deref(),
        &config.database_path,
    )?;

    let db =
        Db::open(&config.database_path).map_err(|e| format!("could not open the database: {e}"))?;

    // R9: no technical setup needed to keep strangers out of the first-run
    // wizard. If the operator did not choose a setup token and setup is not
    // finished, make a one-time, human-typable SETUP CODE for this run and
    // show it where the operator started the server. Never stored.
    let mut config = config;
    if config.setup_token.is_none() {
        let complete = db
            .call(|conn| easy_bitcoin_payments::settings::get_bool(conn, "setup_complete"))
            .await?;
        if !complete {
            let code = easy_bitcoin_payments::auth::generate_setup_code();
            config.setup_token = Some(code.clone());
            config.setup_token_generated = true;
            println!();
            println!("==============================================");
            println!("  SETUP CODE:  {code}");
            println!("  Open the setup page in your browser and type");
            println!("  this code when asked. It changes every time");
            println!("  the server restarts, until setup is finished.");
            println!("==============================================");
            println!();
            tracing::warn!("first-run setup is protected by a one-time setup code (shown above)");
        }
    }

    // Single-container install: run barkd ourselves and pick up its token, so
    // nothing has to be copied or pasted. If there is no token yet (first
    // boot, wallet not created), the wallet stays on the stub until the
    // wizard finishes, after which the server restarts itself once.
    if config.managed_barkd {
        let managed = easy_bitcoin_payments::managed_barkd::install(
            easy_bitcoin_payments::managed_barkd::ManagedBarkd::new(&config.database_path),
        );
        config.barkd_url = managed.api_url();
        match managed.restart(false).await {
            Ok(()) => match managed.token().await {
                Ok(token) => config.barkd_token = Some(token),
                Err(_) => easy_bitcoin_payments::routes::setup::request_restart_after_setup(),
            },
            Err(err) => tracing::error!(error = %err, "managed barkd did not start"),
        }
        tokio::spawn(managed.supervise());
    }

    // BARK INTEGRATION (Phase 1): the wallet layer is a client of the barkd
    // sidecar's REST API (bearer token from `barkd secret show`, passed via
    // EBP_BARKD_TOKEN). Without a token the server still boots on the stub
    // wallet — no payments are observed, nothing can move.
    let wallet: Arc<dyn easy_bitcoin_payments::wallet::WalletAdapter> =
        match easy_bitcoin_payments::wallet::barkd::BarkdWallet::from_app_config(&config) {
            Ok(barkd) => Arc::new(barkd),
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "barkd wallet unavailable; serving with the stub wallet (no payments will be observed)"
                );
                Arc::new(StubWallet::new())
            }
        };

    // The Bitcoin price: one stored value, refreshed in the background from
    // five public sources (see src/rates.rs). Pages and invoice creation only
    // read it, so an outside website can never slow them down or break them.
    let rates = Arc::new(RateService::new());
    rates.clone().spawn(db.clone(), config.rates_timeout_secs);
    let webhooks = Arc::new(WebhookEmitter::new(config.webhook_timeout_secs));

    let state = AppState {
        db: db.clone(),
        config: Arc::new(config.clone()),
        rates,
        wallet: wallet.clone(),
        webhooks: webhooks.clone(),
    };

    // Optional phone alerts (ntfy). Idle unless the merchant turned them on.
    tokio::spawn(easy_bitcoin_payments::alerts::run(
        db.clone(),
        state.wallet.clone(),
    ));

    // Durable webhook outbox dispatcher (beta hardening): delivers events the
    // watcher queued in `webhook_deliveries`, with backoff, surviving
    // restarts. Exactly ONE dispatcher must run.
    let allow_private_webhooks = easy_bitcoin_payments::webhook::allow_private_from_env();
    tokio::spawn(easy_bitcoin_payments::webhook_outbox::run(
        db.clone(),
        webhooks.clone(),
        allow_private_webhooks,
    ));

    // Payment watcher skeleton: polls the wallet adapter on an interval and
    // applies observed payments to invoices. With the stub wallet it observes
    // nothing; the loop, matching, state transitions and webhook emission are
    // all real and unit-tested.
    let watcher = PollingWatcher::new(
        wallet,
        db.clone(),
        webhooks,
        std::time::Duration::from_secs(config.watcher_interval_secs),
        config.renewal_lead_time_secs as i64,
    )
    .with_onchain_rail(config.onchain_rail_enabled)
    .with_onchain_min_confirmations(config.onchain_min_confirmations as i64);
    // Rolling cleanup of operational logs (never invoices or payments).
    tokio::spawn(easy_bitcoin_payments::retention::run(db.clone()));
    let watcher_handle = tokio::spawn(watcher.run());

    // Settlement engine (Phase 1): refreshes VTXOs ahead of their per-VTXO
    // expiry and sweeps the capped till to the merchant's own wallet, while
    // keeping the on-chain emergency-exit reserve. Needs the barkd token;
    // without it (or without a settlement destination) it stays idle.
    match easy_bitcoin_payments::wallet::barkd::BarkdClient::from_app_config(&config) {
        Ok(client) => {
            let engine = easy_bitcoin_payments::wallet::settlement::SettlementEngine::new(
                client,
                db.clone(),
                Arc::new(config.clone()),
            );
            tokio::spawn(engine.run());
        }
        Err(err) => {
            tracing::warn!(error = %err, "settlement engine idle (set EBP_BARKD_TOKEN to enable)")
        }
    }

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    tracing::info!(
        addr = %config.bind_addr,
        database = %config.database_path,
        public_base_url = %config.public_base_url,
        "Easy Bitcoin Payments listening"
    );

    // `with_connect_info` exposes the peer address to the rate limiter.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    watcher_handle.abort();
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}

async fn store_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let sub = args.first().map(String::as_str).unwrap_or("create");
    match sub {
        "create" => {}
        "list" => return store_list().await,
        "rotate-key" => return store_rotate_key(args).await,
        other => return Err(format!("unknown store subcommand: {other}").into()),
    }

    let mut name: Option<String> = None;
    let mut webhook_url: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                name = args.get(i + 1).cloned();
                i += 2;
            }
            "--webhook-url" => {
                webhook_url = args.get(i + 1).cloned();
                i += 2;
            }
            other => return Err(format!("unknown flag: {other}").into()),
        }
    }

    let name = name.ok_or("--name is required")?;
    // SSRF guard (beta hardening): reject loopback/private/link-local and
    // non-http(s) webhook targets. Local development can opt out with
    // EBP_WEBHOOK_ALLOW_PRIVATE=1. See webhook::validate_webhook_url.
    if let Some(url) = webhook_url.as_deref() {
        let allow_private = easy_bitcoin_payments::webhook::allow_private_from_env();
        easy_bitcoin_payments::webhook::validate_webhook_url(url, allow_private)
            .map_err(|e| format!("--webhook-url rejected: {e}"))?;
    }
    let config = Config::load()?;
    // R10: the secret created below is stored encrypted, so this command must
    // use the SAME key as the server (never the ephemeral test key).
    easy_bitcoin_payments::secrets::init(
        config.key_encryption_key.as_deref(),
        &config.database_path,
    )?;
    let db = Db::open(&config.database_path)?;

    let (store, api_key, webhook_secret) = db
        .call(move |conn| store::create(conn, &name, webhook_url.as_deref()))
        .await?;

    println!("Store created: {}", store.name);
    println!("  store id: {}", store.id);
    println!();
    println!("  API key (shown once — store it now):");
    println!("    {api_key}");
    if let Some(secret) = webhook_secret {
        println!();
        println!("  Webhook signing secret (shown once — store it now):");
        println!("    {secret}");
    }
    println!();
    println!("Use it as: Authorization: Bearer <api key>");
    Ok(())
}

/// `ebp store list` — ids and names only (no secrets).
async fn store_list() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    let db = Db::open(&config.database_path)?;
    let rows: Vec<(String, String)> = db
        .call(|conn| {
            let mut stmt = conn.prepare("SELECT id, name FROM stores ORDER BY created_at")?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await?;
    if rows.is_empty() {
        println!("No stores yet.");
    }
    for (id, name) in rows {
        println!("{id}  {name}");
    }
    Ok(())
}

/// `ebp store rotate-key [--store ID]` — lost-key recovery. Revokes every
/// active key of the store and prints one new key, once. With exactly one
/// store the id may be omitted (the beginner case).
async fn store_rotate_key(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use easy_bitcoin_payments::error::AppError;
    let mut store_id: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--store" => {
                store_id = args.get(i + 1).cloned();
                i += 2;
            }
            other => return Err(format!("unknown flag: {other}").into()),
        }
    }
    let config = Config::load()?;
    let db = Db::open(&config.database_path)?;
    let key = db
        .call(move |conn| {
            let id = match store_id {
                Some(id) => id,
                None => {
                    let mut stmt = conn.prepare("SELECT id FROM stores")?;
                    let ids = stmt
                        .query_map([], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()?;
                    match ids.as_slice() {
                        [only] => only.clone(),
                        [] => return Err(AppError::BadRequest("there are no stores yet".into())),
                        _ => {
                            return Err(AppError::BadRequest(
                                "several stores exist: pass --store <ID> (see `ebp store list`)"
                                    .into(),
                            ))
                        }
                    }
                }
            };
            let key = store::rotate_api_key(conn, &id)?;
            easy_bitcoin_payments::audit::record(
                conn,
                &id,
                "store.api_keys_rotated",
                Some("via CLI"),
            )?;
            Ok(key)
        })
        .await?;
    println!("All previous API keys for this store are now revoked.");
    println!();
    println!("  New API key (shown once - store it now):");
    println!("    {key}");
    println!();
    println!("Update the key in your shop plugin / integration, then confirm an");
    println!("order still creates an invoice.");
    Ok(())
}

/// `ebp admin set-password` — set or reset the admin-page password on the
/// server itself (the recovery path; there is deliberately no way to do this
/// remotely). Reads the password from one line of standard input.
async fn admin_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let sub = args.first().map(String::as_str);
    if sub != Some("set-password") && sub != Some("reset-2fa") {
        eprintln!(
            "usage: ebp admin set-password   (type the new password, then press Enter)\n       ebp admin reset-2fa      (turn off two-step sign-in, e.g. after losing your phone)"
        );
        std::process::exit(2);
    }
    let config = Config::load()?;
    // Turning 2FA off does not need the encryption key, so a lost or broken key
    // can never block the lost-phone recovery.
    if sub != Some("reset-2fa") {
        easy_bitcoin_payments::secrets::init(
            config.key_encryption_key.as_deref(),
            &config.database_path,
        )?;
    }
    let db = Db::open(&config.database_path)?;
    if sub == Some("reset-2fa") {
        db.call(|conn| {
            easy_bitcoin_payments::totp::disable(conn)?;
            // Not "declined": the admin page will offer to turn it on again.
            easy_bitcoin_payments::totp::set_declined(conn, false)?;
            if let Some(store) = easy_bitcoin_payments::store::first(conn)? {
                easy_bitcoin_payments::audit::record(
                    conn,
                    &store.id,
                    "admin.2fa_disabled",
                    Some("via CLI"),
                )?;
            }
            Ok(())
        })
        .await?;
        println!("Two-step sign-in is off. Sign in with your password, then turn it on again in Settings.");
        return Ok(());
    }
    eprintln!("New admin password (at least 16 characters; it will be visible as you type):");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let password = line.trim_end_matches(['\r', '\n']).to_string();
    let iterations = config.password_hash_iterations;
    db.call(move |conn| {
        easy_bitcoin_payments::admin_auth::set_password(conn, &password, iterations)?;
        if let Some(store) = easy_bitcoin_payments::store::first(conn)? {
            easy_bitcoin_payments::audit::record(
                conn,
                &store.id,
                "admin.password_changed",
                Some("via CLI"),
            )?;
        }
        Ok(())
    })
    .await?;
    println!("Admin password updated. All signed-in sessions were signed out.");
    Ok(())
}

/// Read a password from the terminal without showing it when possible.
fn read_secret_line(prompt: &str) -> std::io::Result<String> {
    use std::io::IsTerminal;
    eprint!("{prompt}");
    let tty = std::io::stdin().is_terminal();
    let hidden = tty
        && std::process::Command::new("stty")
            .arg("-echo")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    let mut line = String::new();
    let res = std::io::stdin().read_line(&mut line);
    if hidden {
        let _ = std::process::Command::new("stty").arg("echo").status();
        eprintln!();
    }
    res?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

fn usage() -> ! {
    eprintln!("usage: ebp backup verify <FILE>\n       ebp backup restore <FILE> <NEW-FOLDER>");
    std::process::exit(2);
}

fn backup_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use easy_bitcoin_payments::backup;
    let (sub, file) = match (args.first(), args.get(1)) {
        (Some(s), Some(f)) => (s.as_str(), f),
        _ => usage(),
    };
    let bytes = std::fs::read(file).map_err(|e| format!("could not read {file}: {e}"))?;
    let password = read_secret_line("Backup password: ")?;
    let entries = backup::open(&bytes, &password)?;
    match sub {
        "verify" => {
            println!("Backup opened. It contains {} file(s):", entries.len());
            for e in &entries {
                println!("  {}  ({} bytes)", e.name, e.data.len());
            }
            if !entries.iter().any(|e| e.name.starts_with("barkd/")) {
                println!("No wallet files are inside (barkd runs outside EBP). Your recovery words are the wallet backup.");
            }
            Ok(())
        }
        "restore" => {
            let Some(dest) = args.get(2) else { usage() };
            backup::restore_to(&entries, std::path::Path::new(dest))?;
            // Prove the restored database is sound before anyone relies on it.
            let db = std::path::Path::new(dest).join("ebp.sqlite3");
            if db.exists() {
                let conn = rusqlite::Connection::open_with_flags(
                    &db,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
                if check != "ok" {
                    return Err(format!(
                        "the restored database failed its integrity check: {check}"
                    )
                    .into());
                }
            }
            println!("Restored {} file(s) into {dest}.", entries.len());
            println!("Stop the server, put these files into its data folder, and start it again.");
            println!("The webhook secrets inside still need the SAME key-encryption key (EBP_KEY_ENCRYPTION_KEY or ebp.key) as before.");
            Ok(())
        }
        _ => usage(),
    }
}
