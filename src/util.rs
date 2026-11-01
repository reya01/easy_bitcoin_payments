use rand::RngCore;

/// Current UNIX time in whole seconds.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Cryptographically random lowercase hex string of `bytes` bytes.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Prefixed opaque identifier, e.g. `inv_9f2c...`.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", random_hex(12))
}

/// RFC 3339 / ISO 8601 UTC timestamp with second precision.
pub fn to_rfc3339(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = new_id("inv");
        let b = new_id("inv");
        assert!(a.starts_with("inv_"));
        assert_eq!(a.len(), 4 + 24);
        assert_ne!(a, b);
    }

    #[test]
    fn rfc3339_roundtrip() {
        assert_eq!(to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(to_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn now_is_plausible() {
        // After 2024-01-01 and before 2100-01-01.
        let now = now_unix();
        assert!(now > 1_704_067_200);
        assert!(now < 4_102_444_800);
    }
}