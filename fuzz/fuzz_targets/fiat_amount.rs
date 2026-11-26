//! Converting a money amount to sats: never a crash, and an answer is always inside 1 sat to
//! the whole money supply, whatever two numbers come in (NaN, infinity, tiny, huge, negative).
#![no_main]
use easy_bitcoin_payments::{invoice::MAX_INVOICE_SATS, rates::fiat_to_sats};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: [u8; 16]| {
    let amount = f64::from_le_bytes(data[..8].try_into().unwrap());
    let price = f64::from_le_bytes(data[8..].try_into().unwrap());
    if let Ok(sats) = fiat_to_sats(amount, price) {
        assert!(
            (1..=MAX_INVOICE_SATS).contains(&sats),
            "{amount} at {price} gave {sats}"
        );
    }
});
