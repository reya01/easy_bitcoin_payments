use rusqlite::{params, Connection, OptionalExtension};

use crate::auth::{generate_api_key, generate_webhook_secret, hash_secret};
use crate::error::AppError;
use crate::models::Store;
use crate::util::{new_id, now_unix};

/// Create a store plus its first API key.
///
/// Returns the store, the plaintext API key, and (if a webhook URL was given)
/// the plaintext webhook signing secret. Both values are shown to the merchant
/// exactly once and never logged. The API key is stored only as a hash; the
/// webhook secret is stored in plaintext until Phase 1 at-rest encryption lands
/// (see the custody note in `migrations/0001_init.sql`).
pub fn create(
    conn: &Connection,
    name: &str,
    webhook_url: Option<&str>,
) -> Result<(Store, String, Option<String>), AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("store name must not be empty".into()));
    }

    let store_id = new_id("store");
    let created_at = now_unix();

    let api_key = generate_api_key();
    let api_key_hash = hash_secret(&api_key);

    let webhook_secret = webhook_url.map(|_| generate_webhook_secret());
    let webhook_secret_hash = webhook_secret.as_deref().map(hash_secret);
    // R10: the signing secret is stored encrypted (secrets.rs); the plaintext
    // goes back to the caller once and is never persisted.
    let webhook_secret_enc = webhook_secret
        .as_deref()
        .map(|s| crate::secrets::get().encrypt(s, &store_id))
        .transpose()?;

    let tx = conn.unchecked_transaction()?;

    tx.execute(
        "INSERT INTO stores (id, name, webhook_url, webhook_secret_hash, webhook_secret, policy_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, '{}', ?6)",
        params![
            store_id,
            name,
            webhook_url,
            webhook_secret_hash,
            webhook_secret_enc,
            created_at
        ],
    )?;

    tx.execute(
        "INSERT INTO api_keys (id, store_id, key_hash, label, created_at)
         VALUES (?1, ?2, ?3, 'primary', ?4)",
        params![new_id("key"), store_id, api_key_hash, created_at],
    )?;

    tx.commit()?;

    let store = get(conn, &store_id)?
        .ok_or_else(|| AppError::Internal("store disappeared immediately after creation".into()))?;

    Ok((store, api_key, webhook_secret))
}

pub fn get(conn: &Connection, store_id: &str) -> Result<Option<Store>, AppError> {
    conn.query_row(
        "SELECT * FROM stores WHERE id = ?1",
        params![store_id],
        row_to_store,
    )
    .optional()
    .map_err(Into::into)
}

/// Look up a store by the hash of a presented API key. Revoked keys never match.
pub fn find_by_api_key_hash(conn: &Connection, key_hash: &str) -> Result<Option<Store>, AppError> {
    conn.query_row(
        "SELECT s.* FROM stores s
         JOIN api_keys k ON k.store_id = s.id
         WHERE k.key_hash = ?1 AND k.revoked_at IS NULL",
        params![key_hash],
        row_to_store,
    )
    .optional()
    .map_err(Into::into)
}

/// Set (or clear, with `None`) the store's webhook URL.
///
/// A store that has no signing secret yet gets one generated; with
/// `rotate_secret` a new secret replaces the old one. Returns the plaintext
/// secret ONLY when a new one was generated (shown once, like the API key).
/// Clearing the URL keeps the secret so re-enabling does not silently
/// change what the receiver must verify. Callers validate the URL first
/// (`webhook::validate_webhook_url`).
pub fn set_webhook(
    conn: &Connection,
    store_id: &str,
    url: Option<&str>,
    rotate_secret: bool,
) -> Result<Option<String>, AppError> {
    let current = get(conn, store_id)?.ok_or(AppError::NotFound)?;
    let needs_secret = url.is_some() && (rotate_secret || current.webhook_secret.is_none());
    let new_secret = needs_secret.then(generate_webhook_secret);
    match &new_secret {
        Some(secret) => {
            let enc = crate::secrets::get().encrypt(secret, store_id)?;
            conn.execute(
                "UPDATE stores SET webhook_url = ?1, webhook_secret = ?2, webhook_secret_hash = ?3 WHERE id = ?4",
                params![url, enc, hash_secret(secret), store_id],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE stores SET webhook_url = ?1 WHERE id = ?2",
                params![url, store_id],
            )?;
        }
    }
    Ok(new_secret)
}

/// Rotate the store's API key: every active key is revoked and one new key
/// is issued, atomically. Returns the new plaintext key (shown once).
pub fn rotate_api_key(conn: &Connection, store_id: &str) -> Result<String, AppError> {
    if get(conn, store_id)?.is_none() {
        return Err(AppError::NotFound);
    }
    let now = now_unix();
    let key = generate_api_key();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE api_keys SET revoked_at = ?1 WHERE store_id = ?2 AND revoked_at IS NULL",
        params![now, store_id],
    )?;
    tx.execute(
        "INSERT INTO api_keys (id, store_id, key_hash, label, created_at)
         VALUES (?1, ?2, ?3, 'rotated', ?4)",
        params![new_id("key"), store_id, hash_secret(&key), now],
    )?;
    tx.commit()?;
    Ok(key)
}

/// The store's webhook signing secret in plaintext (decrypted on demand;
/// the column and `Store.webhook_secret` hold ciphertext). Never log it.
pub fn webhook_secret_plain(store: &Store) -> Result<Option<String>, AppError> {
    store
        .webhook_secret
        .as_deref()
        .map(|enc| crate::secrets::get().decrypt(enc, &store.id))
        .transpose()
}

/// Startup migration (R10): encrypt any webhook secret still stored as
/// plaintext by an earlier version. Idempotent; returns how many rows changed.
pub fn encrypt_legacy_webhook_secrets(conn: &Connection) -> Result<usize, AppError> {
    let legacy: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, webhook_secret FROM stores
             WHERE webhook_secret IS NOT NULL AND webhook_secret NOT LIKE 'enc:v1:%'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (id, plain) in &legacy {
        let enc = crate::secrets::get().encrypt(plain, id)?;
        conn.execute(
            "UPDATE stores SET webhook_secret = ?1 WHERE id = ?2",
            params![enc, id],
        )?;
    }
    Ok(legacy.len())
}

fn row_to_store(row: &rusqlite::Row<'_>) -> rusqlite::Result<Store> {
    Ok(Store {
        id: row.get("id")?,
        name: row.get("name")?,
        webhook_url: row.get("webhook_url")?,
        webhook_secret_hash: row.get("webhook_secret_hash")?,
        webhook_secret: row.get("webhook_secret")?,
        policy_json: row.get("policy_json")?,
        created_at: row.get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    async fn db() -> Db {
        Db::open(":memory:").unwrap()
    }

    #[tokio::test]
    async fn create_returns_working_api_key() {
        let db = db().await;
        let (store, api_key, secret) = db
            .call(|conn| create(conn, "Test Shop", Some("https://shop.example/hook")))
            .await
            .unwrap();

        assert_eq!(store.name, "Test Shop");
        assert!(api_key.starts_with("ebp_sk_"));
        assert!(secret.is_some());

        let hash = hash_secret(&api_key);
        let found = db
            .call(move |conn| find_by_api_key_hash(conn, &hash))
            .await
            .unwrap()
            .expect("store found by api key");
        assert_eq!(found.id, store.id);
    }

    #[tokio::test]
    async fn unknown_key_does_not_authenticate() {
        let db = db().await;
        db.call(|conn| create(conn, "Shop", None)).await.unwrap();

        let hash = hash_secret("ebp_sk_not_a_real_key");
        let found = db
            .call(move |conn| find_by_api_key_hash(conn, &hash))
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn revoked_keys_stop_working() {
        let db = db().await;
        let (store, api_key, _) = db.call(|conn| create(conn, "Shop", None)).await.unwrap();

        db.call(move |conn| {
            conn.execute(
                "UPDATE api_keys SET revoked_at = 1 WHERE store_id = ?1",
                params![store.id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let hash = hash_secret(&api_key);
        let found = db
            .call(move |conn| find_by_api_key_hash(conn, &hash))
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn empty_name_is_rejected() {
        let db = db().await;
        let err = db.call(|conn| create(conn, "   ", None)).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn webhook_secret_is_encrypted_at_rest_and_decrypts_on_demand() {
        let db = db().await;
        let (store, _key, secret) = db
            .call(|conn| create(conn, "Enc Shop", Some("https://shop.example.com/h")))
            .await
            .unwrap();
        let plain = secret.unwrap();
        // What is in the column is ciphertext, never the secret.
        let stored: String = db
            .call({
                let id = store.id.clone();
                move |conn| {
                    Ok(conn.query_row(
                        "SELECT webhook_secret FROM stores WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )?)
                }
            })
            .await
            .unwrap();
        assert!(stored.starts_with("enc:v1:"));
        assert!(!stored.contains(&plain));
        assert_eq!(webhook_secret_plain(&store).unwrap().unwrap(), plain);
    }

    #[tokio::test]
    async fn legacy_plaintext_secrets_are_encrypted_once_at_startup() {
        let db = db().await;
        let (store, _key, _) = db
            .call(|conn| create(conn, "Legacy", Some("https://shop.example.com/h")))
            .await
            .unwrap();
        let id = store.id.clone();
        // Simulate a pre-encryption row.
        db.call({
            let id = id.clone();
            move |conn| {
                conn.execute(
                    "UPDATE stores SET webhook_secret = 'whsec_legacy' WHERE id = ?1",
                    params![id],
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();
        assert_eq!(
            db.call(|c| encrypt_legacy_webhook_secrets(c))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.call(|c| encrypt_legacy_webhook_secrets(c))
                .await
                .unwrap(),
            0
        );
        let reread = db.call(move |c| get(c, &id)).await.unwrap().unwrap();
        assert_eq!(
            webhook_secret_plain(&reread).unwrap().unwrap(),
            "whsec_legacy"
        );
    }
}
