//! Minimal on-chain helpers: segwit address -> scriptPubKey, and a raw
//! transaction output parser.
//!
//! Why this exists: barkd's `/onchain/transactions` returns the raw
//! transaction (`tx` hex) but no per-output addresses, and EBP must credit an
//! invoice only for an output paying ITS address and only once that
//! transaction is mined (a zero-confirmation payment can still be replaced
//! by the sender via RBF). Matching therefore happens on scriptPubKeys.
//!
//! Only native segwit addresses (bech32 v0, bech32m v1+) are understood,
//! which is all barkd's on-chain wallet issues. Anything else returns
//! `None`, and callers treat that as "cannot match" (fail closed).

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for v in values {
        let top = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ u32::from(*v);
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let b = hrp.as_bytes();
    let mut out: Vec<u8> = b.iter().map(|c| c >> 5).collect();
    out.push(0);
    out.extend(b.iter().map(|c| c & 31));
    out
}

fn convert_5_to_8(data: &[u8]) -> Option<Vec<u8>> {
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut out = Vec::new();
    for v in data {
        acc = (acc << 5) | u32::from(*v);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    // Padding must be fewer than 5 bits, all zero.
    if bits >= 5 || ((acc << (8 - bits)) & 0xff) != 0 {
        return None;
    }
    Some(out)
}

/// The scriptPubKey bytes for a native segwit address, or `None`.
pub fn script_pubkey(address: &str) -> Option<Vec<u8>> {
    // Mixed case is invalid.
    if address.to_lowercase() != address && address.to_uppercase() != address {
        return None;
    }
    let address = address.to_lowercase();
    let sep = address.rfind('1')?;
    let (hrp, rest) = (&address[..sep], &address[sep + 1..]);
    if hrp.is_empty() || rest.len() < 7 {
        return None;
    }
    let data: Vec<u8> = rest
        .bytes()
        .map(|c| CHARSET.iter().position(|x| *x == c).map(|p| p as u8))
        .collect::<Option<Vec<_>>>()?;
    let mut values = hrp_expand(hrp);
    values.extend(&data);
    let check = polymod(&values);
    let version = data[0];
    let expected = match version {
        0 => 1,
        1..=16 => 0x2bc8_30a3,
        _ => return None,
    };
    if check != expected {
        return None;
    }
    let program = convert_5_to_8(&data[1..data.len() - 6])?;
    let valid_len = if version == 0 {
        program.len() == 20 || program.len() == 32
    } else {
        (2..=40).contains(&program.len())
    };
    if !valid_len {
        return None;
    }
    let mut script = vec![if version == 0 { 0 } else { 0x50 + version }];
    script.push(program.len() as u8);
    script.extend(program);
    Some(script)
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.b.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn varint(&mut self) -> Option<u64> {
        let first = self.take(1)?[0];
        match first {
            0xfd => Some(u64::from(u16::from_le_bytes(
                self.take(2)?.try_into().ok()?,
            ))),
            0xfe => Some(u64::from(u32::from_le_bytes(
                self.take(4)?.try_into().ok()?,
            ))),
            0xff => Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            n => Some(u64::from(n)),
        }
    }
}

/// `(amount_sats, scriptPubKey)` for every output of a raw transaction.
pub fn tx_outputs(tx_hex: &str) -> Option<Vec<(u64, Vec<u8>)>> {
    let raw = hex::decode(tx_hex.trim()).ok()?;
    let mut r = Reader { b: &raw, pos: 0 };
    r.take(4)?; // version
    let mut inputs = r.varint()?;
    if inputs == 0 {
        // Segwit marker 0x00 + flag 0x01.
        if r.take(1)?[0] != 1 {
            return None;
        }
        inputs = r.varint()?;
    }
    for _ in 0..inputs {
        r.take(36)?; // outpoint
        let n = r.varint()? as usize;
        r.take(n)?;
        r.take(4)?; // sequence
    }
    let count = r.varint()?;
    let mut out = Vec::new();
    for _ in 0..count {
        let amount = u64::from_le_bytes(r.take(8)?.try_into().ok()?);
        let n = r.varint()? as usize;
        out.push((amount, r.take(n)?.to_vec()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_bip173_p2wpkh() {
        let s = script_pubkey("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4").unwrap();
        assert_eq!(
            hex::encode(s),
            "0014751e76e8199196d454941c45d1b3a323f1433bd6"
        );
    }

    #[test]
    fn decodes_bip350_taproot() {
        let s = script_pubkey("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0")
            .unwrap();
        assert_eq!(
            hex::encode(s),
            "512079be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
    }

    #[test]
    fn rejects_bad_checksum_mixed_case_and_non_segwit() {
        assert!(script_pubkey("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5").is_none());
        assert!(script_pubkey("Bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").is_none());
        assert!(script_pubkey("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2").is_none());
        assert!(script_pubkey("").is_none());
    }

    fn legacy_tx(outputs: &[(u64, &[u8])]) -> String {
        let mut b = vec![1, 0, 0, 0, 1];
        b.extend([7u8; 32]);
        b.extend([0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        b.push(outputs.len() as u8);
        for (amt, script) in outputs {
            b.extend(amt.to_le_bytes());
            b.push(script.len() as u8);
            b.extend(*script);
        }
        b.extend([0, 0, 0, 0]);
        hex::encode(b)
    }

    #[test]
    fn parses_outputs() {
        let a = [0u8, 20]
            .iter()
            .chain([1u8; 20].iter())
            .copied()
            .collect::<Vec<_>>();
        let tx = legacy_tx(&[(1500, &a), (42, &[0x6a])]);
        let outs = tx_outputs(&tx).unwrap();
        assert_eq!(outs, vec![(1500, a), (42, vec![0x6a])]);
    }

    #[test]
    fn parses_segwit_marker_and_rejects_truncation() {
        let mut b = vec![2, 0, 0, 0, 0, 1, 1];
        b.extend([9u8; 32]);
        b.extend([0, 0, 0, 0, 0, 0xfd, 0xff, 0xff, 0xff]);
        b.push(1);
        b.extend(777u64.to_le_bytes());
        b.extend([2, 0x51, 0x51]);
        let full = hex::encode(&b);
        assert_eq!(tx_outputs(&full).unwrap(), vec![(777, vec![0x51, 0x51])]);
        assert!(tx_outputs(&full[..full.len() - 6]).is_none());
        assert!(tx_outputs("zz").is_none());
    }
}
