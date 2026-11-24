//! Invoice number and customer name: labels the owner chooses so payments are
//! easy to match and analyse later (docs/dev/INVOICES.md, "Identifiers for
//! matching and analysis").
//!
//! - The invoice number is free text, at most 40 characters, unique across the
//!   server (upper and lower case count as the same). The default is one more
//!   than the highest purely numeric number so far, starting at 1, so a typed
//!   `INV-0007` does not disturb the counting and a typed `500` carries on
//!   from 501. It is worked out from the stored numbers every time, so a
//!   restored backup can never hand out a number twice.
//! - The customer name is free text, at most 100 characters, not unique.
//!
//! Both live only in the database (and its encrypted backups). Never log them
//! and never put the customer name in a phone alert: alerts pass through a
//! push service.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppError;

/// Longest invoice number accepted, in characters.
pub const MAX_INVOICE_NUMBER_CHARS: usize = 40;
/// Longest customer name accepted, in characters.
pub const MAX_CUSTOMER_NAME_CHARS: usize = 100;
/// Numbers longer than this are labels, not part of the counting (keeps the
/// arithmetic far from overflow).
const MAX_COUNTING_DIGITS: usize = 18;

/// Trim a typed label and check it. Empty means "none".
fn clean(raw: &str, max: usize, what: &str) -> Result<Option<String>, AppError> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.chars().count() > max {
        return Err(AppError::BadRequest(format!(
            "The {what} can be at most {max} characters."
        )));
    }
    if s.chars().any(char::is_control) {
        return Err(AppError::BadRequest(format!(
            "The {what} cannot contain line breaks or other hidden characters."
        )));
    }
    Ok(Some(s.to_string()))
}

/// Check a typed invoice number. Empty means "no number".
pub fn clean_invoice_number(raw: &str) -> Result<Option<String>, AppError> {
    clean(raw, MAX_INVOICE_NUMBER_CHARS, "invoice number")
}

/// Check a typed customer name. Empty means "no name".
pub fn clean_customer_name(raw: &str) -> Result<Option<String>, AppError> {
    clean(raw, MAX_CUSTOMER_NAME_CHARS, "customer name")
}

/// The number a new invoice gets unless the owner types their own: one more
/// than the highest purely numeric number used so far (cancelled and expired
/// invoices included, so a number is never reused), or `1` for the first.
pub fn next_invoice_number(conn: &Connection) -> Result<String, AppError> {
    let mut stmt =
        conn.prepare("SELECT invoice_number FROM invoices WHERE invoice_number IS NOT NULL")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut highest: u64 = 0;
    for row in rows {
        let n = row?;
        if n.is_empty() || n.len() > MAX_COUNTING_DIGITS || !n.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        if let Ok(v) = n.parse::<u64>() {
            highest = highest.max(v);
        }
    }
    Ok((highest + 1).to_string())
}

/// Save (or clear, with empty text) an invoice's number and customer name.
/// A number already used by another invoice is refused.
pub fn set_identifiers(
    conn: &Connection,
    store_id: &str,
    invoice_id: &str,
    invoice_number: &str,
    customer_name: &str,
) -> Result<(), AppError> {
    let number = clean_invoice_number(invoice_number)?;
    let name = clean_customer_name(customer_name)?;
    let changed = conn
        .execute(
            "UPDATE invoices SET invoice_number = ?1, customer_name = ?2
             WHERE id = ?3 AND store_id = ?4",
            params![number, name, invoice_id, store_id],
        )
        .map_err(|e| {
            if is_unique_violation(&e) {
                AppError::Conflict(
                    "That invoice number is already used by another invoice. Choose a different one."
                        .into(),
                )
            } else {
                AppError::from(e)
            }
        })?;
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// An invoice's number and customer name, either of which may be empty.
pub type Identifiers = (Option<String>, Option<String>);

/// Number and customer name for every invoice of a store that has either,
/// keyed by invoice id (for the payments export).
pub fn identifiers_by_invoice(
    conn: &Connection,
    store_id: &str,
) -> Result<HashMap<String, Identifiers>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT id, invoice_number, customer_name FROM invoices
         WHERE store_id = ?1 AND (invoice_number IS NOT NULL OR customer_name IS NOT NULL)",
    )?;
    let rows = stmt.query_map(params![store_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            (
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ),
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, pair) = row?;
        out.insert(id, pair);
    }
    Ok(out)
}

/// Customer names used before, most recently used first, so the form can
/// suggest them and one customer is spelled the same way each time.
pub fn customer_name_suggestions(
    conn: &Connection,
    store_id: &str,
    limit: i64,
) -> Result<Vec<String>, AppError> {
    let mut stmt = conn.prepare(
        "SELECT customer_name FROM invoices
         WHERE store_id = ?1 AND customer_name IS NOT NULL
         GROUP BY customer_name ORDER BY MAX(created_at) DESC, customer_name LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![store_id, limit.clamp(0, 500)], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn is_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::invoice::{insert, NewInvoice};
    use crate::store;
    use crate::util::now_unix;

    fn new_invoice(store_id: &str, id: &str) -> NewInvoice {
        NewInvoice {
            id: id.to_string(),
            store_id: store_id.to_string(),
            order_id: None,
            amount_sats: 25_000,
            fiat_currency: None,
            fiat_amount: None,
            rate: None,
            rate_source: None,
            rate_locked_at: None,
            ark_address: format!("ark1q{id}"),
            expires_at: now_unix() + 1200,
            confs_required: 1,
            metadata_json: "{}".to_string(),
        }
    }

    async fn setup(ids: &[&'static str]) -> (Db, String) {
        let db = Db::open(":memory:").unwrap();
        let ids: Vec<&'static str> = ids.to_vec();
        let store_id = db
            .call(move |conn| {
                let (s, _, _) = store::create(conn, "Test Shop", None)?;
                for id in ids {
                    insert(conn, &new_invoice(&s.id, id))?;
                }
                Ok(s.id)
            })
            .await
            .unwrap();
        (db, store_id)
    }

    #[test]
    fn labels_are_trimmed_and_checked() {
        assert_eq!(
            clean_invoice_number("  INV-7 ").unwrap(),
            Some("INV-7".into())
        );
        assert_eq!(clean_invoice_number("   ").unwrap(), None);
        assert!(clean_invoice_number(&"x".repeat(40)).is_ok());
        assert!(clean_invoice_number(&"x".repeat(41)).is_err());
        assert!(clean_invoice_number("a\nb").is_err());
        assert!(
            clean_customer_name(&"é".repeat(100)).is_ok(),
            "counts characters, not bytes"
        );
        assert!(clean_customer_name(&"é".repeat(101)).is_err());
        assert!(clean_customer_name("Sam\u{0}").is_err());
        assert_eq!(
            clean_customer_name(" Sam's Café ").unwrap(),
            Some("Sam's Café".into())
        );
    }

    #[tokio::test]
    async fn default_number_counts_up_and_skips_labels() {
        let (db, store_id) = setup(&["a", "b", "c", "d", "e"]).await;
        db.call(move |conn| {
            assert_eq!(
                next_invoice_number(conn)?,
                "1",
                "the first invoice is number 1"
            );
            set_identifiers(conn, &store_id, "a", "1", "")?;
            assert_eq!(next_invoice_number(conn)?, "2");
            set_identifiers(conn, &store_id, "b", "INV-0007", "")?;
            assert_eq!(
                next_invoice_number(conn)?,
                "2",
                "a typed label does not disturb the counting"
            );
            set_identifiers(conn, &store_id, "c", "500", "")?;
            assert_eq!(
                next_invoice_number(conn)?,
                "501",
                "a typed 500 carries on from 501"
            );
            set_identifiers(conn, &store_id, "d", &"9".repeat(30), "")?;
            assert_eq!(
                next_invoice_number(conn)?,
                "501",
                "a huge number is a label, not counting"
            );
            // Clearing a number frees nothing below the highest one still used.
            set_identifiers(conn, &store_id, "c", "", "")?;
            assert_eq!(next_invoice_number(conn)?, "2");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_number_points_to_one_invoice_only() {
        let (db, store_id) = setup(&["a", "b"]).await;
        db.call(move |conn| {
            set_identifiers(conn, &store_id, "a", "INV-1", "Sam")?;
            let err = set_identifiers(conn, &store_id, "b", "inv-1", "Sam").unwrap_err();
            assert!(
                matches!(err, AppError::Conflict(_)),
                "case does not make a new number: {err:?}"
            );
            // Saving the same invoice again with its own number is fine.
            set_identifiers(conn, &store_id, "a", "INV-1", "Sam Smith")?;
            // Invoices without a number never clash.
            set_identifiers(conn, &store_id, "b", "", "Sam")?;
            // Another shop's invoice cannot be labelled through this one.
            assert!(matches!(
                set_identifiers(conn, "other-store", "a", "X", "").unwrap_err(),
                AppError::NotFound
            ));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn labels_are_read_back_for_the_export_and_suggestions() {
        let (db, store_id) = setup(&["a", "b", "c"]).await;
        db.call(move |conn| {
            set_identifiers(conn, &store_id, "a", "1", "Sam")?;
            set_identifiers(conn, &store_id, "b", "", "Alex")?;
            let map = identifiers_by_invoice(conn, &store_id)?;
            assert_eq!(map.get("a"), Some(&(Some("1".into()), Some("Sam".into()))));
            assert_eq!(map.get("b"), Some(&(None, Some("Alex".into()))));
            assert!(
                !map.contains_key("c"),
                "an invoice with neither is left out"
            );
            let names = customer_name_suggestions(conn, &store_id, 10)?;
            assert_eq!(names.len(), 2);
            assert!(names.contains(&"Sam".to_string()) && names.contains(&"Alex".to_string()));
            assert!(customer_name_suggestions(conn, "other-store", 10)?.is_empty());
            Ok(())
        })
        .await
        .unwrap();
    }
}
