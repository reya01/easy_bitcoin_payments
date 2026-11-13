use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;

/// Runtime configuration.
///
/// Resolution order (later wins):
///   1. built-in defaults
///   2. optional TOML file (`EBP_CONFIG`, else `./ebp.toml` if present)
///   3. `EBP_*` environment variables
#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_path: String,
    pub public_base_url: String,
    pub default_expiry_minutes: i64,
    pub max_expiry_minutes: i64,
    /// Largest single payment request this server will create (sats). A
    /// sanity limit: a typo or a stolen shop key cannot ask for an absurd sum.
    pub max_invoice_sats: i64,
    /// How many payment requests one shop key may create per minute (0 = no limit).
    pub invoice_rate_limit_per_min: u32,
    pub default_currency: String,
    /// How much the till may hold before it sweeps to an ARK payout address.
    /// Low on purpose: Ark transfers cost (almost) nothing.
    pub hot_wallet_cap_sats: i64,
    /// Same, when the payout address is an ON-CHAIN address. Higher, because
    /// every on-chain withdrawal pays a network fee.
    pub onchain_sweep_cap_sats: i64,
    /// An AUTOMATIC on-chain sweep waits until the network fee rate (sat/vB,
    /// ~3-block target) is at or below this. Manual sweeps ignore it.
    pub onchain_sweep_max_fee_sat_vb: i64,
    pub watcher_interval_secs: u64,
    pub webhook_timeout_secs: u64,
    pub log_level: String,
    /// 64-hex-char (32-byte) key for encrypting webhook secrets at rest
    /// (secrets.rs). Optional: without it a 0600 key file `ebp.key` is
    /// created next to the database. Prefer setting it from outside the data
    /// volume (env `EBP_KEY_ENCRYPTION_KEY`).
    pub key_encryption_key: Option<String>,
    pub admin_token: Option<String>,
    /// Base URL of the barkd sidecar REST API (loopback / internal only).
    pub barkd_url: String,
    /// barkd bearer token. Never logged; env-only (EBP_BARKD_TOKEN).
    pub barkd_token: Option<String>,
    pub checkout_policy_url: Option<String>,
    pub checkout_accent_color: String,
    /// Where the settlement engine sweeps the till to: the merchant's own
    /// wallet. An Ark address on the same ASP (e.g. Arké) sweeps via an Ark
    /// send; a Bitcoin on-chain address sweeps via cooperative offboarding.
    /// Unset (or empty) disables automatic settlement.
    pub settlement_destination: Option<String>,
    /// How often the settlement engine runs.
    pub settlement_interval_secs: u64,
    /// Renew or settle VTXOs this long before their per-VTXO expiry.
    pub renewal_lead_time_secs: u64,
    /// Create a Lightning invoice (ASP gateway) for each checkout invoice.
    pub lightning_receive_enabled: bool,
    /// How long each Bitcoin-price source gets to answer (seconds). The price
    /// is fetched in the background, so this never slows a page down.
    pub rates_timeout_secs: u64,
    /// Deployment mode: `"test"` (default — signet, play money, test
    /// invoices flagged in metadata and excluded from the accounting
    /// export) or `"live"` (mainnet). Set via `mode` in the config file or
    /// the `EBP_MODE` environment variable.
    pub mode: String,
    /// Max requests per client per minute on the public `/i/*` checkout
    /// routes (beta hardening). 0 disables the limiter. See ratelimit.rs.
    pub public_rate_limit_per_min: u32,
    /// Trust `X-Forwarded-For` for the rate-limit client key. Enable ONLY
    /// behind a reverse proxy that overwrites the header.
    pub trust_proxy_headers: bool,
    /// Optional shared secret for the first-run wizard (beta hardening).
    /// When set, every NON-GET `/setup/*` request must carry it in the
    /// `X-EBP-Setup-Token` header; the page and `/setup/status` stay open.
    /// Closes the window where anyone who can reach an unfinished instance
    /// could create the wallet, read the recovery words, or mint the first
    /// store API key. Env-only recommended (`EBP_SETUP_TOKEN`).
    pub setup_token: Option<String>,
    /// True when `setup_token` is the per-boot generated setup code (set by
    /// `main`, never from file/env). Typed codes are then compared
    /// case-/separator-insensitively.
    pub setup_token_generated: bool,
    /// Offer the on-chain rail at checkout. DEFAULT OFF (beta): barkd's
    /// on-chain receive entry in `/wallet/history` is not yet pinned by a
    /// live drill, so on-chain payments cannot be detected with confidence
    /// and a customer could pay an address we would never credit. Off =
    /// no on-chain tab, no on-chain path in the BIP321 URI, `null`
    /// `onchain_address` in the API, and any on-chain payment that still
    /// arrives is parked in `unmatched_payments`, never auto-credited.
    /// Enable with `EBP_ONCHAIN_RAIL_ENABLED=true` once the signet drill in
    /// docs/dev/SIGNET_DRILL.md has passed.
    pub onchain_rail_enabled: bool,
    /// Block confirmations required before an on-chain payment counts as
    /// paid (minimum 1: zero-confirmation payments can be replaced by the
    /// sender via RBF, so they are never credited).
    pub onchain_min_confirmations: u64,
    /// Allow the setup wizard over plain HTTP on a non-local address. Off by
    /// default: the setup code and API keys would cross the network in the
    /// clear. Meant only for a trusted LAN or throwaway testing.
    pub allow_insecure_setup: bool,
    /// Run barkd as a child process of EBP (single-container install). The
    /// wizard then needs no terminal steps. See `managed_barkd`.
    pub managed_barkd: bool,
    /// Look up the newest release for the admin page's update notice (one
    /// anonymous request, cached). EBP never updates itself.
    pub update_check: bool,
    /// PBKDF2 rounds for the admin password (default 600,000; minimum 1,000).
    /// Lowering it weakens the hash — it exists for tests only.
    pub password_hash_iterations: u32,
    /// Seconds a CHANGED payout address waits before it takes effect.
    pub payout_change_delay_secs: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".parse().expect("valid default bind addr"),
            database_path: "./data/ebp.sqlite3".to_string(),
            public_base_url: "http://localhost:8080".to_string(),
            default_expiry_minutes: 20,
            max_expiry_minutes: 1440,
            max_invoice_sats: 100_000_000,
            invoice_rate_limit_per_min: 60,
            default_currency: "EUR".to_string(),
            hot_wallet_cap_sats: 1_000,
            onchain_sweep_cap_sats: 1_000_000,
            onchain_sweep_max_fee_sat_vb: 50,
            watcher_interval_secs: 10,
            webhook_timeout_secs: 10,
            log_level: "info,tower_http=info".to_string(),
            key_encryption_key: None,
            admin_token: None,
            barkd_url: "http://127.0.0.1:3000/api/v1".to_string(),
            barkd_token: None,
            checkout_policy_url: None,
            checkout_accent_color: "#f7931a".to_string(),
            settlement_destination: None,
            settlement_interval_secs: 300,
            renewal_lead_time_secs: 604_800,
            lightning_receive_enabled: true,
            rates_timeout_secs: 4,
            mode: "test".to_string(),
            public_rate_limit_per_min: 120,
            trust_proxy_headers: false,
            setup_token: None,
            setup_token_generated: false,
            onchain_rail_enabled: false,
            onchain_min_confirmations: 1,
            allow_insecure_setup: false,
            managed_barkd: false,
            update_check: true,
            password_hash_iterations: crate::admin_auth::DEFAULT_ITERATIONS,
            payout_change_delay_secs: crate::payout::DEFAULT_DELAY_SECS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("invalid value for {key}: {value}")]
    Invalid { key: String, value: String },
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn mode_defaults_to_test() {
        assert_eq!(Config::default().mode, "test");
    }

    #[test]
    fn mode_accepts_test_and_live_case_insensitively() {
        assert_eq!(Config::parse_mode("live").unwrap(), "live");
        assert_eq!(Config::parse_mode(" TEST ").unwrap(), "test");
    }

    #[test]
    fn mode_rejects_unknown_values() {
        assert!(Config::parse_mode("prod").is_err());
        assert!(Config::parse_mode("").is_err());
    }

    #[test]
    fn mode_flows_through_the_config_file_layer() {
        let file: FileConfig = toml::from_str("mode = \"live\"").unwrap();
        let mut config = Config::default();
        config.apply_file(file).unwrap();
        assert_eq!(config.mode, "live");

        // Unknown modes are rejected at apply time (parse_mode), not TOML time.
        let mut bad_config = Config::default();
        let bad_file: FileConfig = toml::from_str("mode = \"production\"").unwrap();
        assert!(bad_config.apply_file(bad_file).is_err());
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind_addr: Option<String>,
    database_path: Option<String>,
    public_base_url: Option<String>,
    default_expiry_minutes: Option<i64>,
    max_expiry_minutes: Option<i64>,
    default_currency: Option<String>,
    // Retired (the price now always comes from live sources): still accepted
    // in an old config file so an upgrade does not stop the server, then ignored.
    static_rates: Option<toml::Value>,
    hot_wallet_cap_sats: Option<i64>,
    onchain_sweep_cap_sats: Option<i64>,
    onchain_sweep_max_fee_sat_vb: Option<i64>,
    watcher_interval_secs: Option<u64>,
    webhook_timeout_secs: Option<u64>,
    log_level: Option<String>,
    key_encryption_key: Option<String>,
    admin_token: Option<String>,
    barkd_url: Option<String>,
    checkout_policy_url: Option<String>,
    checkout_accent_color: Option<String>,
    settlement_destination: Option<String>,
    settlement_interval_secs: Option<u64>,
    renewal_lead_time_secs: Option<u64>,
    lightning_receive_enabled: Option<bool>,
    rates_provider: Option<toml::Value>,
    rates_timeout_secs: Option<u64>,
    mode: Option<String>,
    public_rate_limit_per_min: Option<u32>,
    trust_proxy_headers: Option<bool>,
    setup_token: Option<String>,
    onchain_rail_enabled: Option<bool>,
    onchain_min_confirmations: Option<u64>,
    allow_insecure_setup: Option<bool>,
    managed_barkd: Option<bool>,
    update_check: Option<bool>,
}

impl Config {
    /// The till cap that applies to a payout destination: Ark addresses use
    /// the low cap, anything else (on-chain) the higher one.
    pub fn sweep_cap_for(&self, destination: &str) -> i64 {
        if is_ark_destination(destination) {
            self.hot_wallet_cap_sats
        } else {
            self.onchain_sweep_cap_sats
        }
    }

    /// May the setup wizard run? Yes over HTTPS, on a loopback address
    /// (reached locally or through an SSH tunnel), or when explicitly
    /// overridden. Judged from `public_base_url`, which is what the merchant
    /// and buyers actually use (a TLS-terminating proxy keeps the app on
    /// HTTP internally).
    pub fn setup_transport_ok(&self) -> bool {
        if self.allow_insecure_setup {
            return true;
        }
        let url = self.public_base_url.trim().to_ascii_lowercase();
        if url.starts_with("https://") {
            return true;
        }
        let Some(rest) = url.strip_prefix("http://") else {
            return false;
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = if let Some(v6) = authority.strip_prefix('[') {
            v6.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        host == "localhost" || host == "::1" || host.starts_with("127.")
    }
}

impl Config {
    /// Load configuration from defaults, optional TOML, and environment.
    pub fn load() -> Result<Config, ConfigError> {
        let explicit = std::env::var("EBP_CONFIG").ok();
        let path = match explicit {
            Some(p) => Some(p),
            None => {
                let default = Path::new("ebp.toml");
                if default.exists() {
                    Some(default.to_string_lossy().to_string())
                } else {
                    None
                }
            }
        };

        let file = match path {
            Some(p) => Some(read_file(&p)?),
            None => None,
        };

        let mut config = Config::default();
        if let Some(file) = file {
            config.apply_file(file)?;
        }
        config.apply_env(|key| std::env::var(key).ok())?;
        // `mode` is parsed here rather than inside `apply_env` so the env
        // override shares the same validation as the config file.
        if let Ok(v) = std::env::var("EBP_MODE") {
            config.mode = Config::parse_mode(&v)?;
        }
        config.validate()?;
        Ok(config)
    }

    fn apply_file(&mut self, file: FileConfig) -> Result<(), ConfigError> {
        if let Some(v) = file.bind_addr {
            self.bind_addr = parse_addr(&v)?;
        }
        if let Some(v) = file.database_path {
            self.database_path = v;
        }
        if let Some(v) = file.public_base_url {
            self.public_base_url = v;
        }
        if let Some(v) = file.default_expiry_minutes {
            self.default_expiry_minutes = v;
        }
        if let Some(v) = file.max_expiry_minutes {
            self.max_expiry_minutes = v;
        }
        if let Some(v) = file.default_currency {
            self.default_currency = v.to_uppercase();
        }
        if file.static_rates.is_some() || file.rates_provider.is_some() {
            eprintln!("note: static_rates and rates_provider in the config file are no longer used; the Bitcoin price now always comes from public price sources");
        }
        if let Some(v) = file.onchain_sweep_max_fee_sat_vb {
            self.onchain_sweep_max_fee_sat_vb = v;
        }
        if let Some(v) = file.onchain_sweep_cap_sats {
            self.onchain_sweep_cap_sats = v;
        }
        if let Some(v) = file.hot_wallet_cap_sats {
            self.hot_wallet_cap_sats = v;
        }
        if let Some(v) = file.watcher_interval_secs {
            self.watcher_interval_secs = v;
        }
        if let Some(v) = file.webhook_timeout_secs {
            self.webhook_timeout_secs = v;
        }
        if let Some(v) = file.log_level {
            self.log_level = v;
        }
        if let Some(v) = file.key_encryption_key {
            self.key_encryption_key = Some(v);
        }
        if let Some(v) = file.admin_token {
            self.admin_token = Some(v);
        }
        if let Some(v) = file.barkd_url {
            self.barkd_url = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = file.checkout_policy_url {
            self.checkout_policy_url = Some(v);
        }
        if let Some(v) = file.checkout_accent_color {
            self.checkout_accent_color = v;
        }
        if let Some(v) = file.settlement_destination {
            self.settlement_destination = Some(v.trim().to_string()).filter(|s| !s.is_empty());
        }
        if let Some(v) = file.settlement_interval_secs {
            self.settlement_interval_secs = v;
        }
        if let Some(v) = file.renewal_lead_time_secs {
            self.renewal_lead_time_secs = v;
        }
        if let Some(v) = file.lightning_receive_enabled {
            self.lightning_receive_enabled = v;
        }
        if let Some(v) = file.rates_timeout_secs {
            self.rates_timeout_secs = v;
        }
        if let Some(v) = file.mode {
            self.mode = Config::parse_mode(&v)?;
        }
        if let Some(v) = file.public_rate_limit_per_min {
            self.public_rate_limit_per_min = v;
        }
        if let Some(v) = file.trust_proxy_headers {
            self.trust_proxy_headers = v;
        }
        if let Some(v) = file.onchain_rail_enabled {
            self.onchain_rail_enabled = v;
        }
        if let Some(v) = file.update_check {
            self.update_check = v;
        }
        if let Some(v) = file.managed_barkd {
            self.managed_barkd = v;
        }
        if let Some(v) = file.allow_insecure_setup {
            self.allow_insecure_setup = v;
        }
        if let Some(v) = file.onchain_min_confirmations {
            self.onchain_min_confirmations = v.max(1);
        }
        if let Some(v) = file.setup_token {
            self.setup_token = Some(v.trim().to_string()).filter(|t| !t.is_empty());
        }
        Ok(())
    }

    /// Parse and validate `mode`: `"test"` or `"live"`
    /// (case-insensitive). Anything else is a configuration error, not a
    /// silent fallback — a typo must never quietly run mainnet.
    fn parse_mode(raw: &str) -> Result<String, ConfigError> {
        let mode = raw.trim().to_lowercase();
        match mode.as_str() {
            "test" | "live" => Ok(mode),
            _ => Err(ConfigError::Invalid {
                key: "mode".to_string(),
                value: raw.to_string(),
            }),
        }
    }

    /// Apply `EBP_*` environment variables. Takes a lookup function so the
    /// behaviour is unit-testable without touching the process environment.
    pub fn apply_env<F>(&mut self, lookup: F) -> Result<(), ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(v) = lookup("EBP_BIND_ADDR") {
            self.bind_addr = parse_addr(&v)?;
        }
        if let Some(v) = lookup("EBP_DATABASE_PATH") {
            self.database_path = v;
        }
        if let Some(v) = lookup("EBP_PUBLIC_BASE_URL") {
            self.public_base_url = v;
        } else if let Some(v) = lookup("RENDER_EXTERNAL_URL") {
            // One-click host (Render) tells us its own https address, so a
            // beginner never has to type it.
            self.public_base_url = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = lookup("EBP_DEFAULT_EXPIRY_MINUTES") {
            self.default_expiry_minutes = parse_int("EBP_DEFAULT_EXPIRY_MINUTES", &v)?;
        }
        if let Some(v) = lookup("EBP_MAX_EXPIRY_MINUTES") {
            self.max_expiry_minutes = parse_int("EBP_MAX_EXPIRY_MINUTES", &v)?;
        }
        if let Some(v) = lookup("EBP_DEFAULT_CURRENCY") {
            self.default_currency = v.to_uppercase();
        }
        if let Some(v) = lookup("EBP_ONCHAIN_SWEEP_MAX_FEE_SAT_VB") {
            self.onchain_sweep_max_fee_sat_vb = parse_int("EBP_ONCHAIN_SWEEP_MAX_FEE_SAT_VB", &v)?;
        }
        if let Some(v) = lookup("EBP_ONCHAIN_SWEEP_CAP_SATS") {
            self.onchain_sweep_cap_sats = parse_int("EBP_ONCHAIN_SWEEP_CAP_SATS", &v)?;
        }
        if let Some(v) = lookup("EBP_HOT_WALLET_CAP_SATS") {
            self.hot_wallet_cap_sats = parse_int("EBP_HOT_WALLET_CAP_SATS", &v)?;
        }
        if let Some(v) = lookup("EBP_WATCHER_INTERVAL_SECS") {
            self.watcher_interval_secs = positive_u64("EBP_WATCHER_INTERVAL_SECS", &v)?;
        }
        if let Some(v) = lookup("EBP_WEBHOOK_TIMEOUT_SECS") {
            self.webhook_timeout_secs = positive_u64("EBP_WEBHOOK_TIMEOUT_SECS", &v)?;
        }
        if let Some(v) = lookup("EBP_LOG_LEVEL") {
            self.log_level = v;
        }
        if let Some(v) = lookup("EBP_KEY_ENCRYPTION_KEY") {
            self.key_encryption_key = Some(v);
        }
        if let Some(v) = lookup("EBP_ADMIN_TOKEN") {
            self.admin_token = Some(v);
        }
        if let Some(v) = lookup("EBP_BARKD_URL") {
            self.barkd_url = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = lookup("EBP_BARKD_TOKEN") {
            // Compose defaults this to an empty string until the wizard
            // provisions the token. Treat empty/whitespace as absent so the
            // server boots on the stub wallet instead of a barkd client with
            // an empty bearer token.
            self.barkd_token = Some(v).filter(|token| !token.trim().is_empty());
        }
        if let Some(v) = lookup("EBP_CHECKOUT_POLICY_URL") {
            self.checkout_policy_url = Some(v);
        }
        if let Some(v) = lookup("EBP_CHECKOUT_ACCENT_COLOR") {
            self.checkout_accent_color = v;
        }
        if let Some(v) = lookup("EBP_SETTLEMENT_DESTINATION") {
            self.settlement_destination = Some(v.trim().to_string()).filter(|s| !s.is_empty());
        }
        if let Some(v) = lookup("EBP_PAYOUT_CHANGE_DELAY_SECS") {
            let n = parse_int("EBP_PAYOUT_CHANGE_DELAY_SECS", &v)?;
            if n < 0 {
                return Err(ConfigError::Invalid {
                    key: "EBP_PAYOUT_CHANGE_DELAY_SECS".to_string(),
                    value: v,
                });
            }
            self.payout_change_delay_secs = n;
        }
        if let Some(v) = lookup("EBP_SETTLEMENT_INTERVAL_SECS") {
            self.settlement_interval_secs = positive_u64("EBP_SETTLEMENT_INTERVAL_SECS", &v)?;
        }
        if let Some(v) = lookup("EBP_RENEWAL_LEAD_TIME_SECS") {
            self.renewal_lead_time_secs = positive_u64("EBP_RENEWAL_LEAD_TIME_SECS", &v)?;
        }
        if let Some(v) = lookup("EBP_RATES_TIMEOUT_SECS") {
            self.rates_timeout_secs = positive_u64("EBP_RATES_TIMEOUT_SECS", &v)?;
        }
        if let Some(v) = lookup("EBP_LIGHTNING_RECEIVE_ENABLED") {
            self.lightning_receive_enabled = parse_bool("EBP_LIGHTNING_RECEIVE_ENABLED", &v)?;
        }
        if let Some(v) = lookup("EBP_PUBLIC_RATE_LIMIT_PER_MIN") {
            let parsed = parse_int("EBP_PUBLIC_RATE_LIMIT_PER_MIN", &v)?;
            self.public_rate_limit_per_min =
                u32::try_from(parsed).map_err(|_| ConfigError::Invalid {
                    key: "EBP_PUBLIC_RATE_LIMIT_PER_MIN".to_string(),
                    value: v.clone(),
                })?;
        }
        if let Some(v) = lookup("EBP_MAX_INVOICE_SATS") {
            self.max_invoice_sats = parse_int("EBP_MAX_INVOICE_SATS", &v)?;
        }
        if let Some(v) = lookup("EBP_INVOICE_RATE_LIMIT_PER_MIN") {
            let parsed = parse_int("EBP_INVOICE_RATE_LIMIT_PER_MIN", &v)?;
            self.invoice_rate_limit_per_min =
                u32::try_from(parsed).map_err(|_| ConfigError::Invalid {
                    key: "EBP_INVOICE_RATE_LIMIT_PER_MIN".to_string(),
                    value: v.clone(),
                })?;
        }
        if let Some(v) = lookup("EBP_ONCHAIN_RAIL_ENABLED") {
            self.onchain_rail_enabled = parse_bool("EBP_ONCHAIN_RAIL_ENABLED", &v)?;
        }
        if let Some(v) = lookup("EBP_UPDATE_CHECK") {
            self.update_check = parse_bool("EBP_UPDATE_CHECK", &v)?;
        }
        if let Some(v) = lookup("EBP_MANAGED_BARKD") {
            self.managed_barkd = parse_bool("EBP_MANAGED_BARKD", &v)?;
        }
        if let Some(v) = lookup("EBP_ALLOW_INSECURE_SETUP") {
            self.allow_insecure_setup = parse_bool("EBP_ALLOW_INSECURE_SETUP", &v)?;
        }
        if let Some(v) = lookup("EBP_ONCHAIN_MIN_CONFIRMATIONS") {
            self.onchain_min_confirmations = positive_u64("EBP_ONCHAIN_MIN_CONFIRMATIONS", &v)?;
        }
        if let Some(v) = lookup("EBP_SETUP_TOKEN") {
            // Compose passes an empty string when unset: treat as absent.
            self.setup_token = Some(v.trim().to_string()).filter(|t| !t.is_empty());
        }
        if let Some(v) = lookup("EBP_TRUST_PROXY_HEADERS") {
            self.trust_proxy_headers = parse_bool("EBP_TRUST_PROXY_HEADERS", &v)?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // The payout address from the server's own configuration gets the same
        // checks as one typed into the admin page: a typo here would send every
        // sweep to a wrong or unspendable address, and a mainnet address on a
        // test server (or the reverse) must never start up.
        if let Some(dest) = self.settlement_destination.as_deref() {
            if let Err(why) = crate::payout::check(dest, &self.mode) {
                return Err(ConfigError::Invalid {
                    key: "settlement_destination".into(),
                    value: format!("{dest} ({why})"),
                });
            }
        }
        if self.max_invoice_sats < 1 || self.max_invoice_sats > crate::invoice::MAX_INVOICE_SATS {
            return Err(ConfigError::Invalid {
                key: "max_invoice_sats (EBP_MAX_INVOICE_SATS)".into(),
                value: self.max_invoice_sats.to_string(),
            });
        }
        if self.default_expiry_minutes < 1 {
            return Err(ConfigError::Invalid {
                key: "default_expiry_minutes".into(),
                value: self.default_expiry_minutes.to_string(),
            });
        }
        if self.max_expiry_minutes < self.default_expiry_minutes {
            return Err(ConfigError::Invalid {
                key: "max_expiry_minutes".into(),
                value: self.max_expiry_minutes.to_string(),
            });
        }
        if self.onchain_sweep_max_fee_sat_vb <= 0 {
            return Err(ConfigError::Invalid {
                key: "onchain_sweep_max_fee_sat_vb".into(),
                value: self.onchain_sweep_max_fee_sat_vb.to_string(),
            });
        }
        if self.onchain_sweep_cap_sats <= 0 {
            return Err(ConfigError::Invalid {
                key: "onchain_sweep_cap_sats".into(),
                value: self.onchain_sweep_cap_sats.to_string(),
            });
        }
        if self.hot_wallet_cap_sats <= 0 {
            return Err(ConfigError::Invalid {
                key: "hot_wallet_cap_sats".into(),
                value: self.hot_wallet_cap_sats.to_string(),
            });
        }
        if self.watcher_interval_secs == 0 {
            return Err(ConfigError::Invalid {
                key: "watcher_interval_secs".into(),
                value: "0".into(),
            });
        }
        if self.webhook_timeout_secs == 0 {
            // From env this is already rejected by positive_u64; the TOML
            // path bypasses that — close the gap here so a zero-timeout
            // webhook client (instant-fail deliveries, no retry) is
            // rejected from every config surface.
            return Err(ConfigError::Invalid {
                key: "webhook_timeout_secs".into(),
                value: "0".into(),
            });
        }
        if self.settlement_interval_secs == 0 {
            return Err(ConfigError::Invalid {
                key: "settlement_interval_secs".into(),
                value: "0".into(),
            });
        }
        if self.renewal_lead_time_secs == 0 {
            return Err(ConfigError::Invalid {
                key: "renewal_lead_time_secs".into(),
                value: "0".into(),
            });
        }
        Ok(())
    }
}

fn read_file(path: &str) -> Result<FileConfig, ConfigError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_string(),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path.to_string(),
        source,
    })
}

fn parse_addr(value: &str) -> Result<SocketAddr, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        key: "bind_addr (EBP_BIND_ADDR), which should look like 127.0.0.1:8080".into(),
        value: value.to_string(),
    })
}

fn parse_int(key: &str, value: &str) -> Result<i64, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        key: key.to_string(),
        value: value.to_string(),
    })
}

/// Parse a boolean from the usual spellings.
fn parse_bool(key: &str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Invalid {
            key: key.to_string(),
            value: value.to_string(),
        }),
    }
}

/// Parse a strictly positive integer. Rejects zero and negatives before the
/// `as u64` cast can silently turn them into near-unbounded intervals.
fn positive_u64(key: &str, value: &str) -> Result<u64, ConfigError> {
    let parsed = parse_int(key, value)?;
    if parsed < 1 {
        return Err(ConfigError::Invalid {
            key: key.to_string(),
            value: value.to_string(),
        });
    }
    Ok(parsed as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn setup_transport_rules() {
        let mut c = super::Config::default();
        for (url, ok) in [
            ("https://pay.example.com", true),
            ("http://localhost:8080", true),
            ("http://127.0.0.1:8080", true),
            ("http://[::1]:8080", true),
            ("http://pay.example.com", false),
            ("http://203.0.113.5:8080", false),
            ("http://localhost.evil.com", false),
            ("ftp://x", false),
        ] {
            c.public_base_url = url.to_string();
            assert_eq!(c.setup_transport_ok(), ok, "{url}");
        }
        c.public_base_url = "http://pay.example.com".into();
        c.allow_insecure_setup = true;
        assert!(c.setup_transport_ok());
    }

    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.default_expiry_minutes, 20);
        assert_eq!(c.hot_wallet_cap_sats, 1_000);
        assert_eq!(c.onchain_sweep_cap_sats, 1_000_000);
        assert_eq!(c.onchain_sweep_max_fee_sat_vb, 50);
        c.validate().expect("defaults validate");
    }

    #[test]
    fn env_overrides_defaults() {
        let mut c = Config::default();
        let env: HashMap<&str, &str> = HashMap::from([
            ("EBP_BIND_ADDR", "127.0.0.1:9999"),
            ("EBP_DEFAULT_CURRENCY", "usd"),
            ("EBP_HOT_WALLET_CAP_SATS", "12345"),
            ("EBP_PUBLIC_BASE_URL", "https://pay.example.com"),
        ]);
        c.apply_env(|k| env.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(c.bind_addr.to_string(), "127.0.0.1:9999");
        assert_eq!(c.default_currency, "USD");
        assert_eq!(c.hot_wallet_cap_sats, 12_345);
        assert_eq!(c.public_base_url, "https://pay.example.com");
    }

    #[test]
    fn invalid_env_is_rejected() {
        let mut c = Config::default();
        let err = c
            .apply_env(|k| (k == "EBP_BIND_ADDR").then(|| "not-an-addr".to_string()))
            .unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn settlement_env_overrides_defaults() {
        let mut c = Config::default();
        let env: HashMap<&str, &str> = HashMap::from([
            ("EBP_SETTLEMENT_DESTINATION", "ark1merchant"),
            ("EBP_SETTLEMENT_INTERVAL_SECS", "60"),
            ("EBP_RENEWAL_LEAD_TIME_SECS", "86400"),
            ("EBP_LIGHTNING_RECEIVE_ENABLED", "false"),
        ]);
        c.apply_env(|k| env.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(c.settlement_destination.as_deref(), Some("ark1merchant"));
        assert_eq!(c.settlement_interval_secs, 60);
        assert_eq!(c.renewal_lead_time_secs, 86_400);
        assert!(!c.lightning_receive_enabled);
    }

    #[test]
    fn invoice_limits_have_safe_defaults_and_can_be_set() {
        let mut c = Config::default();
        assert_eq!(c.max_invoice_sats, 100_000_000);
        assert_eq!(c.invoice_rate_limit_per_min, 60);
        let env: HashMap<&str, &str> = HashMap::from([
            ("EBP_MAX_INVOICE_SATS", "5000000"),
            ("EBP_INVOICE_RATE_LIMIT_PER_MIN", "0"),
        ]);
        c.apply_env(|k| env.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(c.max_invoice_sats, 5_000_000);
        assert_eq!(c.invoice_rate_limit_per_min, 0);
        assert!(c.validate().is_ok());
        c.max_invoice_sats = 0;
        assert!(
            c.validate().is_err(),
            "a cap of zero would refuse everything"
        );
        let mut bad = Config::default();
        assert!(bad
            .apply_env(|k| (k == "EBP_INVOICE_RATE_LIMIT_PER_MIN").then(|| "-1".to_string()))
            .is_err());
    }

    #[test]
    fn validation_rejects_bad_caps() {
        let c = Config {
            hot_wallet_cap_sats: 0,
            ..Config::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn toml_file_parses() {
        let file: FileConfig = toml::from_str(
            r#"
            bind_addr = "0.0.0.0:9000"
            default_currency = "gbp"
            "#,
        )
        .unwrap();
        let mut c = Config::default();
        c.apply_file(file).unwrap();
        assert_eq!(c.bind_addr.to_string(), "0.0.0.0:9000");
        assert_eq!(c.default_currency, "GBP");
    }

    #[test]
    fn retired_price_settings_in_an_old_config_file_are_accepted_and_ignored() {
        let file: FileConfig = toml::from_str(
            r#"
            rates_provider = "static"
            [static_rates]
            EUR = 95000.0
            "#,
        )
        .unwrap();
        let mut c = Config::default();
        c.apply_file(file).unwrap();
        c.validate().expect("still valid");
    }
}

/// Ark addresses are network-prefixed: `ark1…` (mainnet), `tark1…` (signet).
pub fn is_ark_destination(destination: &str) -> bool {
    let d = destination.trim().to_ascii_lowercase();
    d.starts_with("ark1") || d.starts_with("tark1")
}

/// Settings key: the mode (`test` or `live`) the owner chose on the setup
/// page when the wallet was made. Once saved it is the server's mode for
/// good, whatever `EBP_MODE` says, because the wallet is on that network.
pub const SERVER_MODE_KEY: &str = "server_mode";

/// The mode saved by the setup page, if any.
pub fn saved_mode(conn: &rusqlite::Connection) -> Result<Option<String>, crate::error::AppError> {
    Ok(crate::settings::get(conn, SERVER_MODE_KEY)?.filter(|m| m == "test" || m == "live"))
}

pub fn save_mode(conn: &rusqlite::Connection, mode: &str) -> Result<(), crate::error::AppError> {
    crate::settings::set(conn, SERVER_MODE_KEY, mode)
}
