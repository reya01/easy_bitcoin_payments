use std::sync::Arc;

use easy_bitcoin_payments::config::Config;
use easy_bitcoin_payments::db::Db;
use easy_bitcoin_payments::rates::StaticRateProvider;
use easy_bitcoin_payments::routes::{router, AppState};
use easy_bitcoin_payments::store;
use easy_bitcoin_payments::wallet::StubWallet;
use easy_bitcoin_payments::watcher::PollingWatcher;
use easy_bitcoin_payments::webhook::WebhookEmitter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");

    match command {
        "serve" => serve().await,
        "store" => store_command(&args[1..]).await,
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

    let db = Db::open(&config.database_path)?;

    // BARK INTEGRATION: Phase 1 replaces `StubWallet` with a `BarkWallet`
    // adapter backed by bark-wallet / barkd. Nothing else in the server
    // changes — the trait boundary is the whole integration surface.
    let wallet: Arc<dyn easy_bitcoin_payments::wallet::WalletAdapter> =
        Arc::new(StubWallet::new());

    let rates = Arc::new(StaticRateProvider::new(config.static_rates.clone()));
    let webhooks = Arc::new(WebhookEmitter::new(config.webhook_timeout_secs));

    let state = AppState {
        db: db.clone(),
        config: Arc::new(config.clone()),
        rates,
        wallet: wallet.clone(),
        webhooks: webhooks.clone(),
    };

    // Payment watcher skeleton: polls the wallet adapter on an interval and
    // applies observed payments to invoices. With the stub wallet it observes
    // nothing; the loop, matching, state transitions and webhook emission are
    // all real and unit-tested.
    let watcher = PollingWatcher::new(
        wallet,
        db.clone(),
        webhooks,
        std::time::Duration::from_secs(config.watcher_interval_secs),
    );
    let watcher_handle = tokio::spawn(watcher.run());

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;

    tracing::info!(
        addr = %config.bind_addr,
        database = %config.database_path,
        public_base_url = %config.public_base_url,
        "Easy Bitcoin Payments listening"
    );

    axum::serve(listener, app)
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
    if sub != "create" {
        return Err(format!("unknown store subcommand: {sub}").into());
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
    let config = Config::load()?;
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