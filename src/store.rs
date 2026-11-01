use rusqlite::{params, Connection, OptionalExtension};

use crate::auth::{generate_api_key, generate_webhook_secret, hash_secret};
use crate::error::AppError;
use crate::models::Store;
use crate::util::{new_id, now_unix};

/// Create a store plus its first API key.
///
/// Returns the store, the plaintext API key, and (if a webhook URL was given)
/// the plaintext webhook signing secret. Both plaintext values are shown to the
/// merchant exactly once and never stored in plaintext (the API key) or logged.
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

    let tx = conn.unchecked_transaction()?;

    tx.execute(
        "INSERT INTO stores (id, name, webhook_url, webhook_secret_hash, webhook_secret, policy_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, '{}', ?6)",
        params![
            store_id,
            name,
            webhook_url,
            webhook_secret_hash,
            webhook_secret,
            created_at
        ],
    )?;

    tx.execute(
        "INSERT INTO api_keys (id, store_id, key_hash, label, created_at)
         VALUES (?1, ?2, ?3, 'primary', ?4)",
        params![new_id("key"), store_id, api_key_hash, created_at],
    )?;

    tx.commit()?;

    let store = get(conn, &store_id)?.ok_or_else(|| {
        AppError::Internal("store disappeared immediately after creation".into())
    })?;

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
        let (store, api_key, _) = db
            .call(|conn| create(conn, "Shop", None))
            .await
            .unwrap();

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
        let err = db
            .call(|conn| create(conn, "   ", None))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}