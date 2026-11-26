//! The payout address check must never crash on anything pasted, and an address it accepts
//! must be accepted again, unchanged, when checked a second time (what is saved is stable).
#![no_main]
use easy_bitcoin_payments::payout;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    for mode in ["live", "test"] {
        if let Ok(saved) = payout::check(s, mode) {
            assert_eq!(
                payout::check(&saved, mode),
                Ok(saved.clone()),
                "not stable: {s:?}"
            );
            assert!(!saved.is_empty() && saved.is_ascii() && !saved.contains(char::is_whitespace));
        }
    }
    let _ = payout::ark_server_tag(s);
});
