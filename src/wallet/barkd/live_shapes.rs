//! Live-captured barkd response bodies (barkd 0.8.0, signet, 2026-11-01).
//!
//! The three captures that completed shape verification, kept verbatim
//! as the canonical mock-fixture bodies and as shape tests: every
//! client-facing barkd shape is now pinned against a real daemon (the
//! earlier captures — balance, VTXO list, history movements — live on
//! the PATH_* constants in barkd.rs and in `verified_shape_tests`). If a
//! test here fails, a pinned shape changed: re-capture from a live
//! barkd before touching any parser — never edit the body to match the
//! code.

/// GET /api/v1/wallet/connected — a bare boolean under `connected`, no
/// wrapper object.
pub(crate) const BODY_CONNECTED: &str = r#"{"connected":true}"#;

/// GET /api/v1/wallet/mnemonic — only while barkd runs with
/// `--expose-mnemonic`. Twelve words, space-separated, in a single
/// string field (captured from the throwaway signet probe wallet).
pub(crate) const BODY_MNEMONIC: &str = r#"{"mnemonic":"faint toss city dinner satoshi remember voyage mistake surface glide swear hen"}"#;

/// GET /api/v1/fees/lightning/receive?amount_sat=5000 — captured on an
/// EMPTY wallet: `fee_sat` 0 and `vtxos_spent` [] are the empty-wallet
/// values, NOT constants. A wallet with spendable VTXOs can return a
/// nonzero fee and a populated `vtxos_spent`; the parser must take all
/// four fields from the body and assume none of them.
pub(crate) const BODY_LN_FEE_EMPTY_WALLET: &str =
    r#"{"gross_amount_sat":5000,"fee_sat":0,"net_amount_sat":5000,"vtxos_spent":[]}"#;

#[cfg(test)]
mod shape_tests {
    use super::*;

    /// `connected` is the only field and it is a boolean.
    #[test]
    fn connected_is_a_bare_boolean() {
        let v: serde_json::Value = serde_json::from_str(BODY_CONNECTED).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1);
        assert_eq!(v["connected"], true);
    }

    /// One field; twelve space-separated words; nothing else in the body.
    #[test]
    fn mnemonic_is_twelve_words_in_one_field() {
        let v: serde_json::Value = serde_json::from_str(BODY_MNEMONIC).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 1);
        let words = v["mnemonic"].as_str().unwrap();
        assert_eq!(words.split_whitespace().count(), 12);
        assert!(!words.contains('\n') && !words.contains('\t'));
    }

    /// All four fee fields are present; the empty-wallet capture has fee
    /// 0 and no spent VTXOs, and no parser may assume either.
    #[test]
    fn ln_fee_body_has_all_four_fields() {
        let v: serde_json::Value = serde_json::from_str(BODY_LN_FEE_EMPTY_WALLET).unwrap();
        assert_eq!(v["gross_amount_sat"], 5000);
        assert_eq!(v["fee_sat"], 0);
        assert_eq!(v["net_amount_sat"], 5000);
        assert_eq!(v["vtxos_spent"].as_array().unwrap().len(), 0);
        for field in ["gross_amount_sat", "fee_sat", "net_amount_sat"].iter() {
            assert!(
                v[*field].is_i64(),
                "{} must be an integer, not a string",
                field
            );
        }
    }
}
