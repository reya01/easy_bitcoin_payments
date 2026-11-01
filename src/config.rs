use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;

/// Runtime configuration.
///
/// Resolution order (later wins):
///   1. built-in defaults
///   2. optional TOML file (`EBP_CONFIG`, else `./ebp.toml` if present)
///   3. `EBP_*` environment variables
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_path: String,
    pub public_base_url: String,
    pub default_expiry_minutes: i64,
    pub max_expiry_minutes: i64,
    pub default_currency: String,
    /// Static fiat-per-BTC rates used by the Phase 0 rate provider.
    /// Phase 1 replaces this with a live median-of-sources provider.
    pub static_rates: HashMap<String, f64>,
    pub hot_wallet_cap_sats: i64,
    pub watcher_interval_secs: u64,
    pub webhook_timeout_secs: u64,
    pub log_level: String,
    /// Master key for encrypting wallet material and webhook secrets at rest.
    ///
    /// TODO(Phase 1): at-rest encryption is not implemented yet. The field and
    /// the schema columns exist so the migration is additive. Until then the
    /// server refuses to start with a non-empty value only if it is malformed.
    pub key_encryption_key: Option<String>,
    pub admin_token: Option<String>,
    pub checkout_policy_url: Option<String>,
    pub checkout_accent_color: String,
}

impl Default for Config {
    fn default() -> Self {
        let mut static_rates = HashMap::new();
        static_rates.insert("EUR".to_string(), 95_000.0);
        static_rates.insert("USD".to_string(), 88_000.0);
        static_rates.insert("GBP".to_string(), 82_000.0);
        static_rates.insert("CHF".to_string(), 99_000.0);

        Self {
            bind_addr: "0.0.0.0:8080".parse().expect("valid default bind addr"),
            database_path: "./data/ebp.sqlite3".to_string(),
            public_base_url: "http://localhost:8080".to_string(),
            default_expiry_minutes: 20,
            max_expiry_minutes: 1440,
            default_currency: "EUR".to_string(),
            static_rates,
            hot_wallet_cap_sats: 500_000,
            watcher_interval_secs: 10,
            webhook_timeout_secs: 10,
            log_level: "info,tower_http=info".to_string(),
            key_encryption_key: None,
            admin_token: None,
            checkout_policy_url: None,
            checkout_accent_color: "#f7931a".to_string(),
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind_addr: Option<String>,
    database_path: Option<String>,
    public_base_url: Option<String>,
    default_expiry_minutes: Option<i64>,
    max_expiry_minutes: Option<i64>,
    default_currency: Option<String>,
    static_rates: Option<HashMap<String, f64>>,
    hot_wallet_cap_sats: Option<i64>,
    watcher_interval_secs: Option<u64>,
    webhook_timeout_secs: Option<u64>,
    log_level: Option<String>,
    key_encryption_key: Option<String>,
    admin_token: Option<String>,
    checkout_policy_url: Option<String>,
    checkout_accent_color: Option<String>,
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
        if let Some(v) = file.static_rates {
            self.static_rates = v
                .into_iter()
                .map(|(k, v)| (k.to_uppercase(), v))
                .collect();
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
        if let Some(v) = file.checkout_policy_url {
            self.checkout_policy_url = Some(v);
        }
        if let Some(v) = file.checkout_accent_color {
            self.checkout_accent_color = v;
        }
        Ok(())
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
        if let Some(v) = lookup("EBP_HOT_WALLET_CAP_SATS") {
            self.hot_wallet_cap_sats = parse_int("EBP_HOT_WALLET_CAP_SATS", &v)?;
        }
        if let Some(v) = lookup("EBP_WATCHER_INTERVAL_SECS") {
            self.watcher_interval_secs = parse_int("EBP_WATCHER_INTERVAL_SECS", &v)? as u64;
        }
        if let Some(v) = lookup("EBP_WEBHOOK_TIMEOUT_SECS") {
            self.webhook_timeout_secs = parse_int("EBP_WEBHOOK_TIMEOUT_SECS", &v)? as u64;
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
        if let Some(v) = lookup("EBP_CHECKOUT_POLICY_URL") {
            self.checkout_policy_url = Some(v);
        }
        if let Some(v) = lookup("EBP_CHECKOUT_ACCENT_COLOR") {
            self.checkout_accent_color = v;
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ConfigError> {
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
        if self.static_rates.is_empty() {
            return Err(ConfigError::Invalid {
                key: "static_rates".into(),
                value: "empty".into(),
            });
        }
        for (currency, rate) in &self.static_rates {
            if !rate.is_finite() || *rate <= 0.0 {
                return Err(ConfigError::Invalid {
                    key: format!("static_rates.{currency}"),
                    value: rate.to_string(),
                });
            }
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
        key: "bind_addr".into(),
        value: value.to_string(),
    })
}

fn parse_int(key: &str, value: &str) -> Result<i64, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        key: key.to_string(),
        value: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert_eq!(c.default_expiry_minutes, 20);
        assert_eq!(c.hot_wallet_cap_sats, 500_000);
        assert!(c.static_rates.contains_key("EUR"));
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
    fn validation_rejects_bad_caps() {
        let mut c = Config::default();
        c.hot_wallet_cap_sats = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn toml_file_parses() {
        let file: FileConfig = toml::from_str(
            r#"
            bind_addr = "0.0.0.0:9000"
            default_currency = "gbp"
            [static_rates]
            GBP = 80000.0
            "#,
        )
        .unwrap();
        let mut c = Config::default();
        c.apply_file(file).unwrap();
        assert_eq!(c.bind_addr.to_string(), "0.0.0.0:9000");
        assert_eq!(c.default_currency, "GBP");
        assert_eq!(c.static_rates.get("GBP"), Some(&80_000.0));
    }
}