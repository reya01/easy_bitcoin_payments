use std::collections::HashMap;

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
/// Phase 0 ships [`StaticRateProvider`] (configured rates, deterministic and
/// offline). Phase 1 adds a live provider that takes the median of major public
/// APIs and records the source per invoice. The trait is the seam.
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
}