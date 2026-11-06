//! At-rest encryption for secrets the server must be able to read back
//! (today: per-store webhook signing secrets). Closes review finding R10.
//!
//! * Cipher: ChaCha20-Poly1305 (RustCrypto `chacha20poly1305`), a random
//!   96-bit nonce per value, the owning row id bound in as associated data
//!   so a ciphertext cannot be moved to another store's row.
//! * Stored form: `enc:v1:<hex nonce><hex ciphertext+tag>`.
//! * Key (32 bytes), in order of preference:
//!     1. `EBP_KEY_ENCRYPTION_KEY` / `key_encryption_key` = 64 hex chars,
//!        kept OUTSIDE the data volume (env, secret manager). Best: a stolen
//!        database or backup alone reveals nothing.
//!     2. Otherwise a key file `ebp.key` (mode 0600) is created next to the
//!        database on first start. This stops leaks of the SQLite file alone
//!        (SQL injection, an exported DB, a log/ticket attachment) but NOT a
//!        copy of the whole data volume. The encrypted backup download
//!        includes this key, so one file restores everything; prefer
//!        option 1 where you can keep the key outside the volume.
//! * Never in plaintext: nothing here logs a secret or a key.
//! * Not covered: API keys (stored only as SHA-256 hashes), the barkd token
//!   and admin/setup tokens (environment only), the wallet mnemonic (never
//!   stored). Losing the key makes existing webhook secrets unreadable; the
//!   merchant re-issues them with `PUT /api/v1/stores/webhook` (`rotate_secret`).

use std::path::Path;
use std::sync::OnceLock;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

use crate::error::AppError;

/// Prefix of every encrypted value; anything else in a secret column is
/// treated as legacy plaintext by the startup migration only.
pub const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

pub struct SecretBox {
    cipher: ChaCha20Poly1305,
    key: [u8; 32],
}

impl SecretBox {
    pub fn from_key_bytes(key: [u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(&key)),
            key,
        }
    }

    /// The key as hex. ONLY for the encrypted backup bundle (see
    /// `crate::backup`); never log or return it anywhere else.
    pub fn key_hex(&self) -> String {
        hex::encode(self.key)
    }

    /// Encrypt `plaintext`, binding `aad` (the owning row id).
    pub fn encrypt(&self, plaintext: &str, aad: &str) -> Result<String, AppError> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| AppError::Internal("could not encrypt a secret".into()))?;
        Ok(format!("{PREFIX}{}{}", hex::encode(nonce), hex::encode(ct)))
    }

    /// Decrypt a value produced by [`encrypt`](Self::encrypt) with the same `aad`.
    pub fn decrypt(&self, stored: &str, aad: &str) -> Result<String, AppError> {
        let bad = || {
            AppError::Internal(
                "a stored secret could not be decrypted (wrong key or tampered value)".into(),
            )
        };
        let body = stored.strip_prefix(PREFIX).ok_or_else(bad)?;
        let raw = hex::decode(body).map_err(|_| bad())?;
        if raw.len() < NONCE_LEN + 16 {
            return Err(bad());
        }
        let (nonce, ct) = raw.split_at(NONCE_LEN);
        let pt = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| bad())?;
        String::from_utf8(pt).map_err(|_| bad())
    }
}

/// Parse a 64-hex-character key.
pub fn parse_key_hex(s: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(s.trim())
        .map_err(|_| "key_encryption_key must be 64 hex characters (32 bytes)".to_string())?;
    <[u8; 32]>::try_from(raw.as_slice())
        .map_err(|_| "key_encryption_key must be exactly 32 bytes (64 hex characters)".to_string())
}

/// Read the key file at `path`, creating it (mode 0600, random key) if it
/// does not exist. Returns the key and whether it was newly created.
pub fn load_or_create_key_file(path: &Path) -> Result<([u8; 32], bool), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok((parse_key_hex(&text)?, false)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("cannot create key dir: {e}"))?;
            }
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            let mut f = opts
                .open(path)
                .map_err(|e| format!("cannot create key file {}: {e}", path.display()))?;
            f.write_all(hex::encode(key).as_bytes())
                .map_err(|e| format!("cannot write key file: {e}"))?;
            Ok((key, true))
        }
        Err(e) => Err(format!("cannot read key file {}: {e}", path.display())),
    }
}

static BOX: OnceLock<SecretBox> = OnceLock::new();

/// Initialise the process-wide key from config (call once, before serving).
/// Fails startup on a malformed key rather than silently falling back.
pub fn init(configured_key: Option<&str>, database_path: &str) -> Result<(), String> {
    let key = match configured_key.filter(|k| !k.trim().is_empty()) {
        Some(hex_key) => {
            tracing::info!("webhook secrets are encrypted with the configured key_encryption_key");
            parse_key_hex(hex_key)?
        }
        None => {
            let dir = Path::new(database_path)
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let path = dir.join("ebp.key");
            let (key, created) = load_or_create_key_file(&path)?;
            if created {
                tracing::warn!(path = %path.display(), "created a new encryption key file; back it up separately from the database and prefer EBP_KEY_ENCRYPTION_KEY in production");
            } else {
                tracing::info!(path = %path.display(), "webhook secrets are encrypted with the key file");
            }
            key
        }
    };
    let _ = BOX.set(SecretBox::from_key_bytes(key));
    Ok(())
}

/// The process-wide box. `main` calls [`init`] first; if nothing did (unit
/// and integration tests build stores directly) an ephemeral random key is
/// used, which is correct within one process and never persisted.
pub fn get() -> &'static SecretBox {
    BOX.get_or_init(|| {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        SecretBox::from_key_bytes(key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed() -> SecretBox {
        SecretBox::from_key_bytes([7u8; 32])
    }

    #[test]
    fn round_trips_and_binds_the_row_id() {
        let b = boxed();
        let ct = b.encrypt("whsec_abc", "store_1").unwrap();
        assert!(ct.starts_with(PREFIX));
        assert!(!ct.contains("whsec_abc"));
        assert_eq!(b.decrypt(&ct, "store_1").unwrap(), "whsec_abc");
        // Moved to another row: refused.
        assert!(b.decrypt(&ct, "store_2").is_err());
        // Fresh nonce each time.
        assert_ne!(ct, b.encrypt("whsec_abc", "store_1").unwrap());
    }

    #[test]
    fn rejects_tampering_wrong_key_and_plaintext() {
        let b = boxed();
        let ct = b.encrypt("secret", "s").unwrap();
        let mut tampered = ct.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert!(b.decrypt(&tampered, "s").is_err());
        assert!(SecretBox::from_key_bytes([8u8; 32])
            .decrypt(&ct, "s")
            .is_err());
        assert!(
            b.decrypt("whsec_plain", "s").is_err(),
            "legacy plaintext is not silently accepted"
        );
        assert!(b.decrypt("enc:v1:zz", "s").is_err());
    }

    #[test]
    fn key_parsing_is_strict() {
        assert!(parse_key_hex(&"ab".repeat(32)).is_ok());
        assert!(parse_key_hex("abcd").is_err());
        assert!(parse_key_hex(&"zz".repeat(32)).is_err());
    }

    #[test]
    fn key_file_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("ebp-key-{}", crate::util::new_id("t")));
        let path = dir.join("ebp.key");
        let (k1, created) = load_or_create_key_file(&path).unwrap();
        assert!(created);
        let (k2, created2) = load_or_create_key_file(&path).unwrap();
        assert!(!created2);
        assert_eq!(k1, k2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
