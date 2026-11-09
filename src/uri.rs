//! Combined BIP321 payment URIs. Drill 2026-11-02 — real purchase at
//! Second's signet Byte Store (order 56, 1,000 sats, paid over Lightning
//! via the ASP gateway, PAID within seconds): the store's default QR is
//! ONE `bitcoin:` URI — on-chain address in the path, one param per
//! enabled rail (`lightning=`, `ark=`) — which Arke/Noah already parse.

/// sats -> BTC decimal, 8 dp, trailing zeros trimmed: 1000 -> "0.00001".
pub fn sats_to_btc(sats: i64) -> String {
    let whole = sats / 100_000_000;
    let frac = sats % 100_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let mut out = format!("{whole}.{frac:08}");
    while out.ends_with('0') {
        out.pop();
    }
    out
}

/// Combined URI from an invoice's rails. Param order is fixed — amount,
/// lightning, ark — matching the Byte Store's observed URI; a disabled
/// rail contributes no param. Values are inserted verbatim (bech32 and
/// BOLT11 strings are URI-unreserved). BIP321's uppercase variant is
/// deliberately NOT implemented: wallets accept the lowercase form.
pub fn combined(
    amount_sats: i64,
    onchain: Option<&str>,
    lightning: Option<&str>,
    ark: Option<&str>,
) -> String {
    let mut uri = format!(
        "bitcoin:{}?amount={}",
        onchain.unwrap_or(""),
        sats_to_btc(amount_sats)
    );
    if let Some(ln) = lightning {
        uri.push_str("&lightning=");
        uri.push_str(ln);
    }
    if let Some(ark) = ark {
        uri.push_str("&ark=");
        uri.push_str(ark);
    }
    uri
}

/// The Lightning invoice to put inside the combined code.
///
/// The combined code states the order amount. When a Lightning fee was added
/// to the Lightning invoice (the customer pays it), that invoice asks for
/// more than the code says, and some wallets refuse a code whose amounts
/// disagree. In that case Lightning is left out of the combined code; it has
/// its own choice on the checkout page.
pub fn lightning_for_combined(bolt11: Option<&str>, fee_sat: Option<i64>) -> Option<&str> {
    if fee_sat.unwrap_or(0) > 0 {
        None
    } else {
        bolt11
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightning_is_left_out_of_the_combined_code_when_a_fee_makes_amounts_differ() {
        assert_eq!(lightning_for_combined(Some("lnbc1"), None), Some("lnbc1"));
        assert_eq!(
            lightning_for_combined(Some("lnbc1"), Some(0)),
            Some("lnbc1")
        );
        assert_eq!(lightning_for_combined(Some("lnbc1"), Some(3)), None);
        assert_eq!(lightning_for_combined(None, Some(3)), None);
    }

    #[test]
    fn sats_encode_as_trimmed_btc_decimals() {
        assert_eq!(sats_to_btc(1_000), "0.00001"); // Byte Store order 56
        assert_eq!(sats_to_btc(100_000_000), "1");
        assert_eq!(sats_to_btc(1_500_000), "0.015");
        assert_eq!(sats_to_btc(1), "0.00000001");
    }

    #[test]
    fn params_are_ordered_amount_lightning_ark() {
        assert_eq!(
            combined(1_000, Some("bc1q"), Some("lntbs1i"), Some("tark1t")),
            "bitcoin:bc1q?amount=0.00001&lightning=lntbs1i&ark=tark1t"
        );
        assert_eq!(
            combined(1_000, Some("bc1q"), None, Some("tark1t")), // disabled rail: no param
            "bitcoin:bc1q?amount=0.00001&ark=tark1t"
        );
        // On-chain rail off (the beta default): empty path, BIP321 allows it
        // when other payment instructions are present.
        assert_eq!(
            combined(1_000, None, Some("lntbs1i"), Some("tark1t")),
            "bitcoin:?amount=0.00001&lightning=lntbs1i&ark=tark1t"
        );
    }
}
