//! Payment-notice signatures: a correctly signed notice always verifies; changing the body,
//! the secret or one character of the signature never does; and no header crashes the check.
#![no_main]
use easy_bitcoin_payments::webhook::{sign_payload, verify_signature};
use libfuzzer_sys::fuzz_target;

const NOW: i64 = 1_764_000_000;

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let (secret_bytes, body) = rest.split_at(first as usize % (rest.len() + 1));
    let secret = String::from_utf8_lossy(secret_bytes);
    if secret.is_empty() {
        return;
    }

    // Any header at all: no crash. (The input itself, read as a header.)
    let _ = verify_signature(&secret, &String::from_utf8_lossy(data), body, NOW, 300);

    let header = sign_payload(&secret, NOW, body);
    assert!(
        verify_signature(&secret, &header, body, NOW, 300),
        "a good signature failed"
    );
    assert!(
        !verify_signature(&secret, &header, body, NOW + 301, 300),
        "too old, still accepted"
    );

    let mut other_body = body.to_vec();
    other_body.push(b'x');
    assert!(
        !verify_signature(&secret, &header, &other_body, NOW, 300),
        "changed body accepted"
    );
    let other_secret = format!("{secret}x");
    assert!(
        !verify_signature(&other_secret, &header, body, NOW, 300),
        "wrong secret accepted"
    );

    // Flip the last hex digit of the signature.
    let mut flipped = header.clone();
    let last = flipped.pop().unwrap();
    flipped.push(if last == '0' { '1' } else { '0' });
    assert!(
        !verify_signature(&secret, &flipped, body, NOW, 300),
        "changed signature accepted"
    );
});
