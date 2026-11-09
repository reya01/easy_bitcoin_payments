//! The Bitcoin price: ONE source of truth for the whole server.
//!
//! A background task asks five public sources for the price every ten
//! minutes (all at once, a few seconds each), takes the median, and keeps the
//! answer in memory and in the database. Everything else only READS that
//! stored price, so nothing a customer or the owner does can ever wait on, or
//! fail because of, an outside website.
//!
//! The same price serves both jobs: writing "1,000 sats ($0.40)" and turning
//! an order priced in dollars, euros and so on into sats. A price up to ten
//! minutes old can make an order a little cheaper or dearer than the very
//! latest price; that is a small, accepted cost. What is not acceptable is
//! the server failing, so:
//!
//! - one, several or all sources failing is fine: the median of those that
//!   answered is used, and if none answered the last good price stays;
//! - a price is kept for up to 24 hours without a successful refresh, then
//!   dollar amounts disappear and orders priced in a currency are refused
//!   (orders priced in sats keep working);
//! - the last good price is saved, so a restart during an outage keeps it;
//! - there is no built-in fixed table of prices, in testing or anywhere else.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Db;
use crate::error::AppError;
use crate::util::now_unix;

/// The currencies a shop may price in (fiat per 1 BTC). Every source is asked
/// for all of them in a single request.
pub const CURRENCIES: [&str; 7] = ["USD", "EUR", "GBP", "CHF", "CAD", "AUD", "JPY"];

/// How often the sources are asked. Five sources at this pace is about 4,300
/// requests a month each, far below any free-tier limit we know of.
pub const REFRESH_SECS: u64 = 600;

/// A price older than this is no longer used (dollars hidden, orders priced
/// in a currency refused).
pub const MAX_AGE_SECS: i64 = 86_400;

/// A price older than this is "out of date": Home says so, but it is still used.
pub const OLD_AFTER_SECS: i64 = 3_600;

/// A new price more than this far from a price under an hour old waits for
/// the next round (a broken source or a typo, not a real move).
const MAX_JUMP: f64 = 0.25;

const SETTINGS_KEY: &str = "btc_price";
const MAX_BODY_BYTES: usize = 1_000_000;

/// A locked BTC quote for one currency.
#[derive(Debug, Clone)]
pub struct RateQuote {
    pub currency: String,
    /// Fiat units per 1 BTC.
    pub fiat_per_btc: f64,
    pub source: String,
    pub quoted_at: i64,
}

/// One currency's price and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    rate: f64,
    at: i64,
    sources: Vec<String>,
}

/// The stored Bitcoin price. Create it with [`RateService::new`], then call
/// [`RateService::spawn`] once to keep it fresh.
#[derive(Default)]
pub struct RateService {
    prices: RwLock<BTreeMap<String, Entry>>,
}

#[derive(Debug, Default)]
struct MergeOutcome {
    updated: Vec<String>,
    rejected: Vec<String>,
}

/// What one refresh round did.
#[derive(Debug, Default)]
pub struct RefreshReport {
    pub answered: Vec<&'static str>,
    pub failed: Vec<String>,
    pub updated: Vec<String>,
    pub rejected: Vec<String>,
}

impl RateService {
    /// No price yet. Fill it with [`spawn`](Self::spawn).
    pub fn new() -> Self {
        Self::default()
    }

    /// For tests of other parts of the server: a price table that is already
    /// in place, as a successful refresh would have left it. The running
    /// server never calls this; it only ever holds prices it looked up.
    pub fn seeded_for_tests(rates: &[(&str, f64)]) -> Self {
        let service = Self::new();
        {
            let mut prices = service.write();
            for (currency, rate) in rates {
                prices.insert(
                    currency.to_uppercase(),
                    Entry {
                        rate: *rate,
                        at: now_unix(),
                        sources: vec!["test".to_string()],
                    },
                );
            }
        }
        service
    }

    fn read(&self) -> RwLockReadGuard<'_, BTreeMap<String, Entry>> {
        self.prices.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, BTreeMap<String, Entry>> {
        self.prices.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The price of one bitcoin in `currency`, or an error that says why not.
    /// Never waits on the network.
    pub fn quote(&self, currency: &str) -> Result<RateQuote, AppError> {
        let currency = currency.trim().to_uppercase();
        if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(AppError::BadRequest(format!(
                "unsupported currency: {currency}"
            )));
        }
        if !CURRENCIES.contains(&currency.as_str()) {
            return Err(AppError::BadRequest(format!(
                "unsupported currency: {currency} (supported: {})",
                CURRENCIES.join(", ")
            )));
        }
        let now = now_unix();
        let prices = self.read();
        match prices.get(&currency) {
            Some(entry) if now - entry.at <= MAX_AGE_SECS => Ok(RateQuote {
                currency,
                fiat_per_btc: entry.rate,
                source: format!("median({})", entry.sources.join(",")),
                quoted_at: entry.at,
            }),
            _ => Err(AppError::Unavailable(format!(
                "The Bitcoin price for {currency} is not available right now, so a payment priced in {currency} cannot be created. Try again in a few minutes, or price it in sats."
            ))),
        }
    }

    /// Dollars per bitcoin for "(about $0.40)" labels, or `None` when unknown.
    pub fn usd_per_btc(&self) -> Option<f64> {
        self.quote("USD").ok().map(|q| q.fiat_per_btc)
    }

    /// Seconds since the newest price was fetched; `None` = never.
    pub fn newest_age_secs(&self) -> Option<i64> {
        let now = now_unix();
        self.read()
            .values()
            .map(|e| e.at)
            .max()
            .map(|at| (now - at).max(0))
    }

    /// Fold one round of answers in: median per currency, and a price that
    /// jumps far from a recent one waits for the next round.
    fn merge(&self, samples: HashMap<String, Vec<(&'static str, f64)>>, now: i64) -> MergeOutcome {
        let mut outcome = MergeOutcome::default();
        let mut prices = self.write();
        for (currency, answers) in samples {
            let Some(rate) = median(answers.iter().map(|(_, r)| *r).collect()) else {
                continue;
            };
            if let Some(previous) = prices.get(&currency) {
                if now - previous.at < OLD_AFTER_SECS
                    && ((rate / previous.rate) - 1.0).abs() > MAX_JUMP
                {
                    outcome.rejected.push(currency);
                    continue;
                }
            }
            let mut names: Vec<String> = answers.iter().map(|(n, _)| (*n).to_string()).collect();
            names.sort();
            names.dedup();
            prices.insert(
                currency.clone(),
                Entry {
                    rate,
                    at: now,
                    sources: names,
                },
            );
            outcome.updated.push(currency);
        }
        outcome.updated.sort();
        outcome.rejected.sort();
        outcome
    }

    /// Ask every source at once and fold the answers in. Never fails: when
    /// nothing answers, the stored price simply stays.
    pub async fn refresh(&self, client: &reqwest::Client, sources: &[Source]) -> RefreshReport {
        let mut lookups = tokio::task::JoinSet::new();
        for source in sources {
            let client = client.clone();
            let source = source.clone();
            lookups.spawn(async move {
                let result = fetch_one(&client, &source).await;
                (source.kind.name(), result)
            });
        }

        let mut report = RefreshReport::default();
        let mut samples: HashMap<String, Vec<(&'static str, f64)>> = HashMap::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((name, Ok(rates))) => {
                    report.answered.push(name);
                    for (currency, rate) in rates {
                        samples.entry(currency).or_default().push((name, rate));
                    }
                }
                Ok((name, Err(why))) => report.failed.push(format!("{name}: {why}")),
                Err(why) => report.failed.push(format!("a price lookup crashed: {why}")),
            }
        }
        report.answered.sort();

        let outcome = self.merge(samples, now_unix());
        report.updated = outcome.updated;
        report.rejected = outcome.rejected;
        report
    }

    fn snapshot_json(&self) -> String {
        serde_json::to_string(&*self.read()).unwrap_or_default()
    }

    /// Restore a saved price (after a restart). Anything odd is ignored.
    fn load_json(&self, json: &str) {
        let Ok(saved) = serde_json::from_str::<BTreeMap<String, Entry>>(json) else {
            return;
        };
        let now = now_unix();
        let mut prices = self.write();
        for (currency, entry) in saved {
            let sane = CURRENCIES.contains(&currency.as_str())
                && entry.rate.is_finite()
                && entry.rate > 0.0
                && entry.at <= now + 300
                && entry.sources.len() <= 10;
            let newer = prices
                .get(&currency)
                .map_or(true, |have| have.at < entry.at);
            if sane && newer {
                prices.insert(currency, entry);
            }
        }
    }

    /// Keep the price fresh for as long as the server runs: restore the saved
    /// one, then refresh now and every [`REFRESH_SECS`]. A failing or even
    /// crashing round only loses that round.
    pub fn spawn(self: Arc<Self>, db: Db, timeout_secs: u64) {
        tokio::spawn(async move {
            if let Ok(Some(json)) = db.call(|c| crate::settings::get(c, SETTINGS_KEY)).await {
                self.load_json(&json);
            }
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(timeout_secs.max(1)))
                .user_agent(concat!("ebp/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let sources = default_sources();
            let mut tick = tokio::time::interval(Duration::from_secs(REFRESH_SECS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let this = self.clone();
                let (client, sources, db) = (client.clone(), sources.clone(), db.clone());
                let round = tokio::spawn(async move {
                    this.run_round(&client, &sources, &db).await;
                });
                if let Err(why) = round.await {
                    tracing::warn!(error = %why, "price refresh crashed; trying again next round");
                }
            }
        });
    }

    async fn run_round(&self, client: &reqwest::Client, sources: &[Source], db: &Db) {
        let report = self.refresh(client, sources).await;
        if report.answered.is_empty() {
            tracing::warn!(
                failed = %report.failed.join("; "),
                "no price source answered; keeping the last good price"
            );
        } else {
            tracing::debug!(answered = ?report.answered, failed = ?report.failed, "price refreshed");
        }
        if !report.rejected.is_empty() {
            tracing::warn!(
                currencies = ?report.rejected,
                "a new price was far from the last one; waiting for the next round to confirm"
            );
        }
        if !report.updated.is_empty() {
            let json = self.snapshot_json();
            let _ = db
                .call(move |c| crate::settings::set(c, SETTINGS_KEY, &json))
                .await;
        }
    }
}

/// The public sources. Each answers for all [`CURRENCIES`] in one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    MempoolSpace,
    BlockchainInfo,
    CoinGecko,
    Kraken,
    Coinbase,
}

/// A source and the address to ask.
#[derive(Debug, Clone)]
pub struct Source {
    pub kind: SourceKind,
    pub url: String,
}

/// The five sources asked every round.
pub fn default_sources() -> Vec<Source> {
    [
        SourceKind::MempoolSpace,
        SourceKind::BlockchainInfo,
        SourceKind::CoinGecko,
        SourceKind::Kraken,
        SourceKind::Coinbase,
    ]
    .into_iter()
    .map(|kind| Source {
        kind,
        url: kind.default_url(),
    })
    .collect()
}

impl SourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::MempoolSpace => "mempool.space",
            Self::BlockchainInfo => "blockchain.info",
            Self::CoinGecko => "coingecko",
            Self::Kraken => "kraken",
            Self::Coinbase => "coinbase",
        }
    }

    fn default_url(self) -> String {
        match self {
            Self::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            Self::BlockchainInfo => "https://blockchain.info/ticker".to_string(),
            Self::CoinGecko => format!(
                "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies={}",
                CURRENCIES
                    .iter()
                    .map(|c| c.to_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Kraken => format!(
                "https://api.kraken.com/0/public/Ticker?pair={}",
                CURRENCIES
                    .iter()
                    .map(|c| format!("XBT{c}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Coinbase => "https://api.coinbase.com/v2/exchange-rates?currency=BTC".to_string(),
        }
    }

    /// Pull every usable fiat-per-BTC price out of one source's JSON answer.
    fn parse(self, body: &Value) -> HashMap<String, f64> {
        let mut out = HashMap::new();
        for currency in CURRENCIES {
            let rate = match self {
                // {"time": 1700000000, "USD": 65432.1, "EUR": 60111.0, ...}
                Self::MempoolSpace => body.get(currency).and_then(Value::as_f64),
                // {"USD": {"15m": ..., "last": 65432.1, ...}, ...}
                Self::BlockchainInfo => body
                    .get(currency)
                    .and_then(|v| v.get("last"))
                    .and_then(Value::as_f64),
                // {"bitcoin": {"usd": 65432.1, "eur": 60111.0}}
                Self::CoinGecko => body
                    .get("bitcoin")
                    .and_then(|v| v.get(currency.to_lowercase()))
                    .and_then(Value::as_f64),
                // {"error": [], "result": {"XXBTZUSD": {"c": ["65432.1", "0.01"], ...}, ...}}
                Self::Kraken => body
                    .get("result")
                    .and_then(Value::as_object)
                    .and_then(|pairs| {
                        pairs
                            .iter()
                            .find(|(key, _)| key.contains("XBT") && key.ends_with(currency))
                            .map(|(_, value)| value)
                    })
                    .and_then(|v| v.get("c"))
                    .and_then(|c| c.get(0))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok()),
                // {"data": {"currency": "BTC", "rates": {"USD": "65432.1", ...}}}
                Self::Coinbase => body
                    .get("data")
                    .and_then(|d| d.get("rates"))
                    .and_then(|r| r.get(currency))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok()),
            };
            if let Some(rate) = rate.filter(|r| r.is_finite() && *r > 0.0) {
                out.insert(currency.to_string(), rate);
            }
        }
        out
    }
}

/// Median of the sample, `None` when it is empty. Even-sized samples average
/// their two middle values; sorting keeps one wild source from dragging the
/// price anywhere but the middle.
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

/// Ask one source. `Err` is a plain-words reason for the log.
async fn fetch_one(
    client: &reqwest::Client,
    source: &Source,
) -> Result<HashMap<String, f64>, String> {
    let response = client
        .get(&source.url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err("the answer was unexpectedly large".to_string());
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let rates = source.kind.parse(&body);
    if rates.is_empty() {
        return Err("no usable prices in the answer".to_string());
    }
    Ok(rates)
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

    // Upper bound = the entire bitcoin money supply in sats
    // (`invoice::MAX_INVOICE_SATS`, also enforced at insert). Rejecting here
    // gives the fiat path the same early 400 the `sats` path has instead of
    // letting an absurd conversion travel as far as the insert.
    if !sats.is_finite() || sats < 1.0 || sats > crate::invoice::MAX_INVOICE_SATS as f64 {
        return Err(AppError::BadRequest(
            "amount is out of the supported range".into(),
        ));
    }

    Ok(sats.round() as i64)
}

/// "1,000 sats ($0.40)" when a dollar price is known, else "1,000 sats".
pub fn sats_with_usd(sats: i64, usd_per_btc: Option<f64>) -> String {
    let mut digits = sats.abs().to_string();
    let mut i = digits.len();
    while i > 3 {
        i -= 3;
        digits.insert(i, ',');
    }
    let sign = if sats < 0 { "-" } else { "" };
    match usd_per_btc.filter(|r| r.is_finite() && *r > 0.0) {
        None => format!("{sign}{digits} sats"),
        Some(rate) => {
            let usd = sats.abs() as f64 / 1e8 * rate;
            let amount = if usd > 0.0 && usd < 0.01 {
                "<$0.01".to_string()
            } else {
                let cents = (usd * 100.0).round() as i64;
                let mut whole = (cents / 100).to_string();
                let mut j = whole.len();
                while j > 3 {
                    j -= 3;
                    whole.insert(j, ',');
                }
                format!("${whole}.{:02}", cents % 100)
            };
            format!("{sign}{digits} sats ({amount})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Json, Router};
    use serde_json::json;

    #[test]
    fn fiat_amounts_beyond_the_money_supply_are_rejected() {
        // 95,000 per BTC: 1e12 fiat ≈ 1.05e7 BTC ≈ 1.05e15 sats is still
        // possible, but 1e13 fiat exceeds 21M BTC.
        assert!(fiat_to_sats(1e13, 95_000.0).is_err());
        assert!(fiat_to_sats(42.0, 95_000.0).is_ok());
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
    fn every_source_answer_shape_parses() {
        let mempool = json!({"time": 1_700_000_000, "USD": 65_432.1, "EUR": 60_111.0});
        let p = SourceKind::MempoolSpace.parse(&mempool);
        assert_eq!(p.get("USD"), Some(&65_432.1));
        assert_eq!(p.get("EUR"), Some(&60_111.0));
        assert_eq!(
            p.get("GBP"),
            None,
            "time is not a currency, missing ones are skipped"
        );

        let blockchain = json!({
            "USD": {"15m": 65_000.0, "last": 65_432.1, "symbol": "$"},
            "CHF": {"last": 58_000.0}
        });
        let p = SourceKind::BlockchainInfo.parse(&blockchain);
        assert_eq!(p.get("USD"), Some(&65_432.1));
        assert_eq!(p.get("CHF"), Some(&58_000.0));

        let gecko = json!({"bitcoin": {"usd": 65_432.1, "eur": 60_111.0}});
        let p = SourceKind::CoinGecko.parse(&gecko);
        assert_eq!(p.get("USD"), Some(&65_432.1));
        assert_eq!(p.get("EUR"), Some(&60_111.0));

        let kraken = json!({"error": [], "result": {
            "XXBTZUSD": {"c": ["65432.10", "0.5"]},
            "XXBTZEUR": {"c": ["60111.00", "0.5"]},
            "XBTCHF": {"c": ["58000.00", "0.5"]}
        }});
        let p = SourceKind::Kraken.parse(&kraken);
        assert_eq!(p.get("USD"), Some(&65_432.1));
        assert_eq!(p.get("EUR"), Some(&60_111.0));
        assert_eq!(p.get("CHF"), Some(&58_000.0));
        assert_eq!(p.get("JPY"), None);

        let coinbase = json!({"data": {"currency": "BTC", "rates": {"USD": "65432.1", "EUR": "60111.0", "XYZ": "1"}}});
        let p = SourceKind::Coinbase.parse(&coinbase);
        assert_eq!(p.get("USD"), Some(&65_432.1));
        assert_eq!(p.get("EUR"), Some(&60_111.0));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn junk_answers_give_no_prices() {
        for kind in [
            SourceKind::MempoolSpace,
            SourceKind::BlockchainInfo,
            SourceKind::CoinGecko,
            SourceKind::Kraken,
            SourceKind::Coinbase,
        ] {
            assert!(kind.parse(&json!({})).is_empty());
            assert!(kind.parse(&json!([1, 2, 3])).is_empty());
            assert!(kind.parse(&json!("html error page")).is_empty());
        }
        // zero, negative and non-numbers never become a price
        assert!(SourceKind::MempoolSpace
            .parse(&json!({"USD": 0, "EUR": -5.0, "GBP": "x"}))
            .is_empty());
    }

    #[test]
    fn quotes_follow_the_stored_price() {
        let service = RateService::seeded_for_tests(&[("EUR", 95_000.0), ("usd", 88_000.0)]);
        let quote = service.quote("eur").unwrap();
        assert_eq!(quote.currency, "EUR");
        assert_eq!(quote.fiat_per_btc, 95_000.0);
        assert!(quote.source.contains("median("));
        assert_eq!(service.usd_per_btc(), Some(88_000.0));
    }

    #[test]
    fn no_price_means_unavailable_not_a_guess() {
        let service = RateService::new();
        assert!(matches!(
            service.quote("USD").unwrap_err(),
            AppError::Unavailable(_)
        ));
        assert_eq!(service.usd_per_btc(), None);
        assert_eq!(service.newest_age_secs(), None);
    }

    #[test]
    fn unknown_or_malformed_currencies_are_bad_requests() {
        let service = RateService::seeded_for_tests(&[("USD", 1.0)]);
        for bad in ["XYZ", "us", "euro", "EU1", ""] {
            assert!(
                matches!(service.quote(bad).unwrap_err(), AppError::BadRequest(_)),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_price_older_than_a_day_is_no_longer_used() {
        let service = RateService::seeded_for_tests(&[("USD", 50_000.0)]);
        service.write().get_mut("USD").unwrap().at -= MAX_AGE_SECS + 5;
        assert!(matches!(
            service.quote("USD").unwrap_err(),
            AppError::Unavailable(_)
        ));
        assert_eq!(service.usd_per_btc(), None);
        assert!(service.newest_age_secs().unwrap() > MAX_AGE_SECS);
    }

    fn samples(pairs: &[(&'static str, f64)]) -> HashMap<String, Vec<(&'static str, f64)>> {
        let mut m = HashMap::new();
        m.insert(
            "USD".to_string(),
            pairs.iter().map(|(n, r)| (*n, *r)).collect(),
        );
        m
    }

    #[test]
    fn sats_are_written_with_dollars_when_a_price_is_known() {
        assert_eq!(sats_with_usd(1_000, Some(40_000.0)), "1,000 sats ($0.40)");
        assert_eq!(sats_with_usd(1_000, None), "1,000 sats");
        assert_eq!(sats_with_usd(0, Some(40_000.0)), "0 sats ($0.00)");
        assert_eq!(sats_with_usd(5, Some(40_000.0)), "5 sats (<$0.01)");
        assert_eq!(
            sats_with_usd(250_000_000, Some(88_000.0)),
            "250,000,000 sats ($220,000.00)"
        );
        assert_eq!(sats_with_usd(1_000, Some(f64::NAN)), "1,000 sats");
    }

    #[test]
    fn the_median_of_whoever_answered_is_used() {
        let service = RateService::new();
        let now = now_unix();
        let out = service.merge(samples(&[("a", 100.0), ("b", 101.0), ("c", 900.0)]), now);
        assert_eq!(out.updated, vec!["USD".to_string()]);
        assert_eq!(
            service.usd_per_btc(),
            Some(101.0),
            "one wild source is ignored"
        );

        let single = RateService::new();
        single.merge(samples(&[("only", 77.0)]), now);
        assert_eq!(single.usd_per_btc(), Some(77.0), "one answer is enough");
    }

    #[test]
    fn nothing_answering_keeps_the_last_good_price() {
        let service = RateService::seeded_for_tests(&[("USD", 50_000.0)]);
        let out = service.merge(HashMap::new(), now_unix());
        assert!(out.updated.is_empty());
        assert_eq!(service.usd_per_btc(), Some(50_000.0));
    }

    #[test]
    fn a_sudden_jump_waits_unless_the_old_price_is_already_old() {
        let service = RateService::seeded_for_tests(&[("USD", 50_000.0)]);
        let now = now_unix();
        let out = service.merge(samples(&[("a", 90_000.0), ("b", 91_000.0)]), now);
        assert_eq!(out.rejected, vec!["USD".to_string()]);
        assert_eq!(service.usd_per_btc(), Some(50_000.0));

        // a normal move is taken at once
        let out = service.merge(samples(&[("a", 52_000.0)]), now);
        assert_eq!(out.updated, vec!["USD".to_string()]);
        assert_eq!(service.usd_per_btc(), Some(52_000.0));

        // after an hour without a refresh, a big move is believed
        service.write().get_mut("USD").unwrap().at -= OLD_AFTER_SECS + 5;
        let out = service.merge(samples(&[("a", 90_000.0)]), now);
        assert_eq!(out.updated, vec!["USD".to_string()]);
        assert_eq!(service.usd_per_btc(), Some(90_000.0));
    }

    #[test]
    fn the_saved_price_survives_a_restart_and_junk_is_ignored() {
        let service = RateService::seeded_for_tests(&[("USD", 61_000.0), ("EUR", 56_000.0)]);
        let saved = service.snapshot_json();

        let restarted = RateService::new();
        restarted.load_json(&saved);
        assert_eq!(restarted.usd_per_btc(), Some(61_000.0));

        let junk = RateService::new();
        junk.load_json("not json");
        junk.load_json(
            r#"{"USD":{"rate":-1.0,"at":1,"sources":[]},"XYZ":{"rate":5.0,"at":1,"sources":[]}}"#,
        );
        assert_eq!(junk.usd_per_btc(), None);
        assert_eq!(junk.newest_age_secs(), None);
    }

    async fn mock_server() -> String {
        let app = Router::new()
            .route(
                "/mempool",
                get(|| async { Json(json!({"USD": 100_000.0, "EUR": 90_000.0})) }),
            )
            .route(
                "/coinbase",
                get(|| async {
                    Json(json!({"data": {"rates": {"USD": "100200.0", "EUR": "90200.0"}}}))
                }),
            )
            .route(
                "/broken",
                get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "oops") }),
            )
            .route("/junk", get(|| async { "<html>not json</html>" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        base
    }

    #[tokio::test]
    async fn a_refresh_uses_the_sources_that_answer_and_survives_the_rest() {
        let base = mock_server().await;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let sources = vec![
            Source {
                kind: SourceKind::MempoolSpace,
                url: format!("{base}/mempool"),
            },
            Source {
                kind: SourceKind::Coinbase,
                url: format!("{base}/coinbase"),
            },
            Source {
                kind: SourceKind::Kraken,
                url: format!("{base}/broken"),
            },
            Source {
                kind: SourceKind::CoinGecko,
                url: format!("{base}/junk"),
            },
            Source {
                kind: SourceKind::BlockchainInfo,
                // nothing listens here: connection refused
                url: "http://127.0.0.1:1/ticker".to_string(),
            },
        ];

        let service = RateService::new();
        let report = service.refresh(&client, &sources).await;
        assert_eq!(report.answered, vec!["coinbase", "mempool.space"]);
        assert_eq!(report.failed.len(), 3, "{:?}", report.failed);
        assert_eq!(report.updated, vec!["EUR".to_string(), "USD".to_string()]);
        assert_eq!(service.usd_per_btc(), Some(100_100.0));
        assert_eq!(service.quote("EUR").unwrap().fiat_per_btc, 90_100.0);

        // everything down: the price stays, nothing is reported as updated
        let dead = vec![Source {
            kind: SourceKind::Kraken,
            url: format!("{base}/broken"),
        }];
        let report = service.refresh(&client, &dead).await;
        assert!(report.answered.is_empty());
        assert!(report.updated.is_empty());
        assert_eq!(service.usd_per_btc(), Some(100_100.0));
    }
}
