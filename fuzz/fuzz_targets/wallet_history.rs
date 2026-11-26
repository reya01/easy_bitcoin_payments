//! Wallet history entries, as the wallet (barkd) sends them: odd or hostile JSON must never
//! crash the reader, and anything it turns into a payment has a positive amount and an address.
#![no_main]
use easy_bitcoin_payments::wallet::barkd::HistoryEntry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(entry) = serde_json::from_slice::<HistoryEntry>(data) else {
        return;
    };
    if let Some(payments) = entry.into_wallet_payments() {
        for p in payments {
            assert!(p.amount_sats > 0, "a payment of {} sats", p.amount_sats);
            assert!(!p.address.is_empty(), "a payment with no address");
        }
    }
});
