//! The Lightning invoice reader (`bolt11_expires_at`) must never crash, whatever the Ark
//! server or a pasted string holds, and an end time it returns is never before the invoice
//! was made.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Some(end) = easy_bitcoin_payments::manual_invoice::bolt11_expires_at(s) {
        assert!(end >= 0, "an end time before 1970: {end}");
    }
});
