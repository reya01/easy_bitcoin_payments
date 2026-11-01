use std::collections::HashMap;
use std::time::Duration;

use crate::error::AppError;
use crate::util::now_unix;

/// A locked BTC quote for one currency.
#[derive(Debug, Clone)]
pub struct RateQuote {
    pub currency: String,
    /// Fiat units per 1 BTC.
    pub fiat_per_btc: f64,
    pub source: String,
    pub quoted_at: i64,
}

/// Source of BTC/fiat rates.
///
/// [`StaticRateProvider`] serves configured rates (deterministic, offline —
/// the default, and what tests use). [`LiveRateProvider`] takes the median
/// of public spot APIs and records the source per invoice. The trait is the
/// seam.
pub trait RateProvider: Send + Sync {
    fn quote(&self, currency: &str) -> Result<RateQuote, AppError>;
}

pub struct StaticRateProvider {
    rates: HashMap<String, f64>,
}

impl StaticRateProvider {
    pub fn new(rates: HashMap<String, f64>) -> Self {
        Self {
            rates: rates
                .into_iter()
                .map(|(currency, rate)| (currency.to_uppercase(), rate))
                .collect(),
        }
    }
}

impl RateProvider for StaticRateProvider {
    fn quote(&self, currency: &str) -> Result<RateQuote, AppError> {
        let currency = currency.trim().to_uppercase();
        let rate = *self.rates.get(&currency).ok_or_else(|| {
            AppError::BadRequest(format!("unsupported currency: {currency}"))
        })?;

        if !rate.is_finite() || rate <= 0.0 {
            return Err(AppError::Internal(
                "configured BTC rate is not a positive number".into(),
            ));
        }

        Ok(RateQuote {
            currency,
            fiat_per_btc: rate,
            source: "static(config)".to_string(),
            quoted_at: now_unix(),
        })
    }
}

/// A public BTC/fiat spot source polled by [`LiveRateProvider`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateSourceKind {
    MempoolSpace,
    BlockchainInfo,
    CoinGecko,
}

/// The public sources the live provider polls by default.
pub const DEFAULT_RATE_SOURCES: [RateSourceKind; 3] = [
    RateSourceKind::MempoolSpace,
    RateSourceKind::BlockchainInfo,
    RateSourceKind::CoinGecko,
];

/// How long a live quote is reused before the sources are polled again, so
/// invoice bursts do not hammer the public APIs.
const QUOTE_CACHE_SECS: i64 = 30;

impl RateSourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::MempoolSpace => "mempool.space",
            Self::BlockchainInfo => "blockchain.info",
            Self::CoinGecko => "coingecko",
        }
    }

    fn url_for(self, currency: &str) -> String {
        match self {
            Self::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            Self::BlockchainInfo => "https://blockchain.info/ticker".to_string(),
            Self::CoinGecko => format!(
                "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies={currency}"
            ),
        }
    }

    /// Pull the fiat-per-BTC rate for `currency` (uppercase) out of one
    /// source's JSON body.
    fn parse(self, body: &serde_json::Value, currency: &str) -> Option<f64> {
        match self {
            // {"time": 1700000000, "USD": 65432.1, "EUR": 60111.0, ...}
            Self::MempoolSpace => body.get(currency).and_then(|value| value.as_f64()),
            // {"USD": {"15m": ..., "last": 65432.1, "buy": ..., ...}, ...}
            Self::BlockchainInfo => body
                .get(currency)
                .and_then(|value| value.get("last"))
                .and_then(|value| value.as_f64()),
            // {"bitcoin": {"usd": 65432.1, "eur": 60111.0}}
            Self::CoinGecko => body
                .get("bitcoin")
                .and_then(|value| value.get(currency.to_lowercase()))
                .and_then(|value| value.as_f64()),
        }
    }
}

/// Median of the sample, `None` when it is empty. Even-sized samples average
/// their two middle values; sorting keeps one wild source from dragging the
/// quote anywhere but the middle.
fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        Some(values[mid])
    } else {
        Some((values[mid - 1] + values[mid]) / 2.0)
    }
}

/// Ask one source for its rate. `Err` carries the transport/HTTP failure;
/// `Ok(None)` means the source answered but had no rate for the currency.
async fn fetch_from_source(
    client: &reqwest::Client,
    source: RateSourceKind,
    currency: &str,
) -> Result<Option<f64>, String> {
    let response = client
        .get(source.url_for(currency))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    Ok(source.parse(&body, currency))
}

/// Build one quote: ask every source (each fetch carries the client's
/// timeout), keep the sane answers, take their median. Fails only when every
/// source fails.
async fn fetch_live_quote(
    client: &reqwest::Client,
    sources: &[RateSourceKind],
    currency: &str,
) -> Result<RateQuote, AppError> {
    let mut rates: Vec<f64> = Vec::new();
    let mut used: Vec<&'static str> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for source in sources {
        match fetch_from_source(client, *source, currency).await {
            Ok(Some(rate)) if rate.is_finite() && rate > 0.0 => {
                rates.push(rate);
                used.push(source.name());
            }
            Ok(_) => failures.push(format!("{}: no usable {currency} rate", source.name())),
            Err(err) => failures.push(format!("{}: {err}", source.name())),
        }
    }

    let Some(fiat_per_btc) = median(rates) else {
        return Err(AppError::Internal(format!(
            "all rate sources failed ({})",
            failures.join("; ")
        )));
    };

    Ok(RateQuote {
        currency: currency.to_string(),
        fiat_per_btc,
        source: format!("median({})", used.join(",")),
        quoted_at: now_unix(),
    })
}

/// Live rate provider: the median of public BTC/fiat spot APIs.
///
/// The [`RateProvider`] trait is synchronous (the invoice path calls it
/// without an await point), so the HTTP work runs on a dedicated worker
/// thread with its own single-threaded runtime — the same bridge-thread
/// shape the wallet layer uses. Every source fetch carries its own timeout,
/// a quote fails only when every source fails, and the last quote is reused
/// for [`QUOTE_CACHE_SECS`] so invoice bursts do not hammer the APIs.
pub struct LiveRateProvider {
    requests: std::sync::mpsc::Sender<LiveRatesRequest>,
}

/// One quote request on its way to the rates worker.
struct LiveRatesRequest {
    currency: String,
    reply: std::sync::mpsc::Sender<Result<RateQuote, AppError>>,
}

impl LiveRateProvider {
    /// Live provider polling the default public sources.
    pub fn new(timeout_secs: u64) -> Self {
        Self::with_sources(DEFAULT_RATE_SOURCES.to_vec(), timeout_secs)
    }

    /// Live provider polling `sources`, each fetch capped at `timeout_secs`.
    pub fn with_sources(sources: Vec<RateSourceKind>, timeout_secs: u64) -> Self {
        let (requests, inbox) = std::sync::mpsc::channel::<LiveRatesRequest>();

        std::thread::Builder::new()
            .name("ebp-rates".to_string())
            .spawn(move || rates_worker(inbox, sources, timeout_secs))
            .expect("could not spawn the rates worker thread");

        Self { requests }
    }
}

impl RateProvider for LiveRateProvider {
    fn quote(&self, currency: &str) -> Result<RateQuote, AppError> {
        let currency = currency.trim().to_uppercase();
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(AppError::BadRequest(format!("unsupported currency: {currency}")));
        }

        let (reply, responses) = std::sync::mpsc::channel();
        self.requests
            .send(LiveRatesRequest { currency, reply })
            .map_err(|_| AppError::Internal("rates worker stopped".into()))?;

        responses.recv().map_err(|_| AppError::Internal("rates worker stopped".into()))?
    }
}

/// The rates worker: owns a single-threaded runtime, answers quote requests
/// until the provider (and its sender) is dropped, and reuses the last quote
/// for [`QUOTE_CACHE_SECS`].
fn rates_worker(
    mut inbox: std::sync::mpsc::Receiver<LiveRatesRequest>,
    sources: Vec<RateSourceKind>,
    timeout_secs: u64,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            tracing::error!(error = %err, "could not start the rates runtime");
            return;
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .user_agent(concat!("ebp/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut cache: Option<(String, RateQuote)> = None;

    while let Ok(request) = inbox.recv() {
        if let Some((currency, quote)) = &cache {
            if *currency == request.currency && now_unix() - quote.quoted_at < QUOTE_CACHE_SECS {
                let _ = request.reply.send(Ok(quote.clone()));
                continue;
            }
        }

        let quote = runtime.block_on(fetch_live_quote(&client, &sources, &request.currency));
        if let Ok(fetched) = &quote {
            cache = Some((request.currency.clone(), fetched.clone()));
        }
        let _ = request.reply.send(quote);
    }
}

/// Convert a fiat amount to sats at a locked rate, rounding to the nearest sat.
pub fn fiat_to_sats(fiat_amount: f64, fiat_per_btc: f64) -> Result<i64, AppError> {
    if !fiat_amount.is_finite() || fiat_amount <= 0.0 {
        return Err(AppError::BadRequest(
            "fiat amount must be a positive number".into(),
        ));
    }
    if !fiat_per_btc.is_finite() || fiat_per_btc <= 0.0 {
        return Err(AppError::Internal("invalid BTC rate".into()));
    }

    let sats = (fiat_amount / fiat_per_btc) * 100_000_000.0;

    if !sats.is_finite() || sats < 1.0 || sats > i64::MAX as f64 {
        return Err(AppError::BadRequest(
            "amount is out of the supported range".into(),
        ));
    }

    Ok(sats.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> StaticRateProvider {
        StaticRateProvider::new(HashMap::from([
            ("EUR".to_string(), 95_000.0),
            ("USD".to_string(), 88_000.0),
        ]))
    }

    #[test]
    fn quotes_are_case_insensitive() {
        let quote = provider().quote("eur").unwrap();
        assert_eq!(quote.currency, "EUR");
        assert_eq!(quote.fiat_per_btc, 95_000.0);
        assert_eq!(quote.source, "static(config)");
    }

    #[test]
    fn unknown_currency_is_a_bad_request() {
        let err = provider().quote("XYZ").unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn fiat_conversion_rounds_to_nearest_sat() {
        // 42.00 EUR at 95,000 EUR/BTC = 0.000442105263 BTC = 44,210.53 sats -> 44,211
        assert_eq!(fiat_to_sats(42.0, 95_000.0).unwrap(), 44_211);
        // 1 EUR at 100,000 EUR/BTC = 1,000 sats exactly
        assert_eq!(fiat_to_sats(1.0, 100_000.0).unwrap(), 1_000);
    }

    #[test]
    fn tiny_amounts_are_rejected_rather_than_rounded_to_zero() {
        assert!(fiat_to_sats(0.0000001, 95_000.0).is_err());
    }

    #[test]
    fn non_positive_amounts_are_rejected() {
        assert!(fiat_to_sats(0.0, 95_000.0).is_err());
        assert!(fiat_to_sats(-5.0, 95_000.0).is_err());
        assert!(fiat_to_sats(f64::NAN, 95_000.0).is_err());
        assert!(fiat_to_sats(10.0, 0.0).is_err());
    }

    #[test]
    fn median_of_odd_samples_is_the_middle_value() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![1000.0, 101.0, 100.0]), Some(101.0));
    }

    #[test]
    fn median_of_even_samples_averages_the_middle_two() {
        assert_eq!(median(vec![4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn median_of_empty_and_single_samples() {
        assert_eq!(median(Vec::new()), None);
        assert_eq!(median(vec![1.0]), Some(1.0));
    }

    #[test]
    fn mempool_space_payload_parses() {
        let body = serde_json::json!({
            "time": 1_700_000_000,
            "USD": 65_432.1,
            "EUR": 60_111.0
        });
        assert_eq!(RateSourceKind::MempoolSpace.parse(&body, "EUR"), Some(60_111.0));
        assert_eq!(RateSourceKind::MempoolSpace.parse(&body, "GBP"), None);
    }

    #[test]
    fn blockchain_info_payload_parses() {
        let body = serde_json::json!({
            "USD": {"15m": 65_000.0, "last": 65_432.1, "buy": 65_400.0, "sell": 65_500.0, "symbol": "$"},
            "EUR": {"15m": 60_000.0, "last": 60_111.0, "buy": 60_100.0, "sell": 60_200.0, "symbol": "EUR"}
        });
        assert_eq!(RateSourceKind::BlockchainInfo.parse(&body, "USD"), Some(65_432.1));
        assert_eq!(RateSourceKind::BlockchainInfo.parse(&body, "CHF"), None);
    }

    #[test]
    fn coingecko_payload_parses() {
        let body = serde_json::json!({"bitcoin": {"usd": 65_432.1, "eur": 60_111.0}});
        assert_eq!(RateSourceKind::CoinGecko.parse(&body, "USD"), Some(65_432.1));
        assert_eq!(RateSourceKind::CoinGecko.parse(&body, "eur"), Some(60_111.0));
        assert_eq!(RateSourceKind::CoinGecko.parse(&body, "GBP"), None);
    }

    #[test]
    fn live_provider_rejects_malformed_currencies_before_fetching() {
        let provider = LiveRateProvider::new(5);
        for bad in ["us", "euro", "EU1", ""] {
            assert!(matches!(provider.quote(bad).unwrap_err(), AppError::BadRequest(_)));
        }
    }
}