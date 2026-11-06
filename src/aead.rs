//! ChaCha20-Poly1305 (RFC 8439) through `ring`, which the TLS stack already
//! ships, so EBP needs no separate cipher crate. The byte layout is the
//! standard one (12-byte nonce, 16-byte tag appended to the ciphertext), so
//! data written by the earlier RustCrypto-based version opens unchanged; the
//! RFC 8439 test vector below pins that.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};

pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;

fn key(key: &[u8; 32]) -> LessSafeKey {
    // A 32-byte key is always valid for ChaCha20-Poly1305.
    LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, key).expect("32-byte key"))
}

/// Returns ciphertext || tag.
pub fn seal(k: &[u8; 32], nonce: &[u8; NONCE_LEN], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut buf = plaintext.to_vec();
    key(k)
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(*nonce),
            Aad::from(aad),
            &mut buf,
        )
        .expect("sealing in memory cannot fail");
    buf
}

/// `None` for a wrong key, wrong AAD, or any tampering (indistinguishable).
pub fn open(
    k: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext_and_tag: &[u8],
) -> Option<Vec<u8>> {
    let mut buf = ciphertext_and_tag.to_vec();
    let n = key(k)
        .open_in_place(
            Nonce::assume_unique_for_key(*nonce),
            Aad::from(aad),
            &mut buf,
        )
        .ok()?
        .len();
    buf.truncate(n);
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 8439 section 2.8.2.
    #[test]
    fn matches_the_rfc_8439_test_vector() {
        let k: [u8; 32] = core::array::from_fn(|i| 0x80 + i as u8);
        let nonce: [u8; 12] = hex::decode("070000004041424344454647")
            .unwrap()
            .try_into()
            .unwrap();
        let aad = hex::decode("50515253c0c1c2c3c4c5c6c7").unwrap();
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let out = seal(&k, &nonce, &aad, pt);
        let expect_tag = "1ae10b594f09e26a7e902ecbd0600691";
        assert_eq!(hex::encode(&out[out.len() - TAG_LEN..]), expect_tag);
        assert_eq!(hex::encode(&out[..16]), "d31a8d34648e60db7b86afbc53ef7ec2");
        assert_eq!(open(&k, &nonce, &aad, &out).unwrap(), pt);
    }

    #[test]
    fn wrong_aad_or_tampering_fails() {
        let k = [7u8; 32];
        let n = [1u8; 12];
        let mut ct = seal(&k, &n, b"row-1", b"secret");
        assert!(open(&k, &n, b"row-2", &ct).is_none());
        ct[0] ^= 1;
        assert!(open(&k, &n, b"row-1", &ct).is_none());
    }
}
