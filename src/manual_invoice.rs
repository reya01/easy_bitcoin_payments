//! Invoices the owner makes by hand on the Invoices tab (docs/dev/INVOICES.md).
//!
//! A manual invoice is an ordinary row in `invoices` with `source = 'manual'`
//! (migration 0016), so the watcher, the paid / part-paid / overpaid rules,
//! the late-payment window and the payments export treat it exactly like a
//! shop order. What differs: the owner gives it a note, an invoice number and
//! a customer name (src/invoice_ident.rs), it stays open for days instead of
//! minutes, no shop webhook is sent for it, and the Invoices tab can ask for a
//! fresh Lightning invoice once the old one has run out.
//!
//! The feature is in testing and off until the owner turns it on in Settings.
//! The note and customer name are never logged and never put in phone alerts.

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AppError;
use crate::models::Invoice;

/// Settings key: "1" when the Invoices tab is turned on.
pub const ENABLED_KEY: &str = "invoices_enabled";
/// Longest note ("what it is for") accepted, in characters.
pub const MAX_NOTE_CHARS: usize = 200;
/// How long an invoice can stay open, in days. The Ark address works for all
/// of it; a Lightning invoice lasts only as long as the Ark server allows.
pub const OPEN_DAYS_CHOICES: [i64; 4] = [1, 7, 14, 30];
pub const DEFAULT_OPEN_DAYS: i64 = 7;
/// A fresh Lightning invoice is only made this long after the old one ran
/// out, so a payment that was on its way at the last moment is never left
/// pointing at a replaced invoice.
pub const FRESH_LIGHTNING_GRACE_SECS: i64 = 10 * 60;

pub fn is_enabled(conn: &Connection) -> Result<bool, AppError> {
    Ok(crate::settings::get(conn, ENABLED_KEY)?.as_deref() == Some("1"))
}

pub fn set_enabled(conn: &Connection, on: bool) -> Result<(), AppError> {
    crate::settings::set(conn, ENABLED_KEY, if on { "1" } else { "0" })
}

/// Check the typed note. Empty means "no note".
pub fn clean_note(raw: &str) -> Result<Option<String>, AppError> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.chars().count() > MAX_NOTE_CHARS {
        return Err(AppError::BadRequest(format!(
            "The note can be at most {MAX_NOTE_CHARS} characters."
        )));
    }
    if s.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "The note cannot contain line breaks or other hidden characters.".into(),
        ));
    }
    Ok(Some(s.to_string()))
}

/// Is this invoice number already used (upper and lower case count as the
/// same)? Checked before the wallet is asked for anything; the unique index
/// still has the last word.
pub fn number_taken(conn: &Connection, number: &str) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM invoices WHERE invoice_number = ?1 COLLATE NOCASE)",
        params![number],
        |row| row.get(0),
    )?)
}

/// Mark a just-made invoice as manual and store its labels. If the number was
/// taken in the meantime the invoice is removed again (it was never shown to
/// anyone) and the plain "already used" message is returned.
pub fn finish_new(
    conn: &Connection,
    store_id: &str,
    invoice_id: &str,
    note: Option<&str>,
    invoice_number: &str,
    customer_name: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE invoices SET source = 'manual', note = ?2 WHERE id = ?1 AND store_id = ?3",
        params![invoice_id, note, store_id],
    )?;
    if let Err(err) = crate::invoice_ident::set_identifiers(
        conn,
        store_id,
        invoice_id,
        invoice_number,
        customer_name,
    ) {
        conn.execute(
            "DELETE FROM invoices WHERE id = ?1 AND store_id = ?2 AND paid_sats = 0",
            params![invoice_id, store_id],
        )?;
        return Err(err);
    }
    Ok(())
}

pub fn is_manual(conn: &Connection, invoice_id: &str) -> Result<bool, AppError> {
    let source: Option<String> = conn
        .query_row(
            "SELECT source FROM invoices WHERE id = ?1",
            params![invoice_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(source.as_deref() == Some("manual"))
}

/// One manual invoice with everything the Invoices tab shows.
#[derive(Debug, Clone)]
pub struct ManualInvoice {
    pub invoice: Invoice,
    pub note: Option<String>,
    pub invoice_number: Option<String>,
    pub customer_name: Option<String>,
    pub bolt11: Option<String>,
}

/// The invoice row plus its current Lightning invoice, followed by `$rest`
/// (a fixed string; every value is a bound parameter).
macro_rules! manual_select {
    ($rest:literal) => {
        concat!(
            "SELECT invoices.*,
                (SELECT bolt11 FROM ln_invoices l WHERE l.invoice_id = invoices.id) AS bolt11
             FROM invoices ",
            $rest
        )
    };
}

fn row_to_manual(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManualInvoice> {
    Ok(ManualInvoice {
        invoice: crate::invoice::row_to_invoice(row)?,
        note: row.get("note")?,
        invoice_number: row.get("invoice_number")?,
        customer_name: row.get("customer_name")?,
        bolt11: row.get("bolt11")?,
    })
}

pub fn get(
    conn: &Connection,
    store_id: &str,
    invoice_id: &str,
) -> Result<Option<ManualInvoice>, AppError> {
    let sql = manual_select!("WHERE id = ?1 AND store_id = ?2 AND source = 'manual'");
    Ok(conn
        .query_row(sql, params![invoice_id, store_id], row_to_manual)
        .optional()?)
}

/// Escape `%`, `_` and `\` so typed text is matched literally by LIKE.
fn like_pattern(q: &str) -> String {
    let mut out = String::from("%");
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Manual invoices, newest first, optionally only those whose number or
/// customer name contains `query` (upper and lower case alike).
pub fn list(
    conn: &Connection,
    store_id: &str,
    query: Option<&str>,
    limit: i64,
) -> Result<Vec<ManualInvoice>, AppError> {
    let q = query.map(str::trim).filter(|q| !q.is_empty());
    let sql = manual_select!(
        "WHERE store_id = ?1 AND source = 'manual'
           AND (?2 IS NULL OR invoice_number LIKE ?2 ESCAPE '\\'
                OR customer_name LIKE ?2 ESCAPE '\\')
         ORDER BY created_at DESC, rowid DESC LIMIT ?3"
    );
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![store_id, q.map(like_pattern), limit.clamp(1, 500)],
        row_to_manual,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Every manual invoice of a store, oldest first, for the invoices CSV.
pub fn export_all(conn: &Connection, store_id: &str) -> Result<Vec<ManualInvoice>, AppError> {
    let sql = manual_select!(
        "WHERE store_id = ?1 AND source = 'manual'
         ORDER BY created_at ASC, rowid LIMIT ?2"
    );
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![store_id, crate::invoice::EXPORT_MAX_ROWS],
        row_to_manual,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// When a BOLT11 Lightning invoice stops working (unix seconds): its creation
/// time plus its expiry field (BOLT11 says 3600 seconds when the field is
/// missing). This is read from the invoice itself, after the Ark server set
/// it, so the payment request shows the real time and not the one EBP asked
/// for. `None` if it cannot be read; the signature is not checked (the
/// invoice came from this server's own wallet).
pub fn bolt11_expires_at(bolt11: &str) -> Option<i64> {
    let lower = bolt11.trim().to_ascii_lowercase();
    let s = lower.strip_prefix("lightning:").unwrap_or(&lower);
    let decoded = crate::payout::decode_bech32(s)?;
    if decoded.bech32m || !decoded.hrp.starts_with("ln") {
        return None;
    }
    const TIMESTAMP: usize = 7; // 35 bits
    const SIGNATURE: usize = 104; // 520 bits
    let data = &decoded.data;
    if data.len() < TIMESTAMP + SIGNATURE {
        return None;
    }
    let body = &data[..data.len() - SIGNATURE];
    let created = body[..TIMESTAMP]
        .iter()
        .fold(0i64, |acc, &v| (acc << 5) | i64::from(v));
    let mut expiry = 3600i64;
    let mut i = TIMESTAMP;
    while i + 3 <= body.len() {
        let tag = body[i];
        let len = (usize::from(body[i + 1]) << 5) | usize::from(body[i + 2]);
        i += 3;
        if i + len > body.len() {
            return None;
        }
        // 'x' (5-bit value 6) is the expiry in seconds.
        if tag == 6 {
            if len == 0 || len > 12 {
                return None;
            }
            expiry = body[i..i + len]
                .iter()
                .fold(0i64, |acc, &v| (acc << 5) | i64::from(v));
        }
        i += len;
    }
    created.checked_add(expiry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::invoice::{insert, NewInvoice};
    use crate::store;
    use crate::util::now_unix;

    // Made for this test (bech32 with a good checksum, zero signature):
    // created 1764000000, expiry field 10800 seconds.
    const WITH_EXPIRY: &str = "lntbs10u1p5jfqgqdqydxyzxqr23sqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqpmn3vw";
    // The same without an expiry field: BOLT11's default of 3600 seconds.
    const NO_EXPIRY: &str = "lnbc11p5jfqgqdqydxyzqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq7lwwyz";

    #[test]
    fn lightning_expiry_is_read_from_the_invoice() {
        assert_eq!(bolt11_expires_at(WITH_EXPIRY), Some(1_764_000_000 + 10_800));
        assert_eq!(
            bolt11_expires_at(&WITH_EXPIRY.to_uppercase()),
            Some(1_764_010_800)
        );
        assert_eq!(
            bolt11_expires_at(&format!("lightning:{WITH_EXPIRY}")),
            Some(1_764_010_800)
        );
        assert_eq!(bolt11_expires_at(NO_EXPIRY), Some(1_764_000_000 + 3_600));
        // A broken checksum, an Ark address and rubbish are not guessed at.
        let mut broken = WITH_EXPIRY.to_string();
        broken.pop();
        broken.push('q');
        assert_eq!(bolt11_expires_at(&broken), None);
        assert_eq!(bolt11_expires_at("tark1qqqqqq"), None);
        assert_eq!(bolt11_expires_at(""), None);
    }

    #[test]
    fn notes_are_trimmed_and_checked() {
        assert_eq!(
            clean_note("  Bathroom tap repair ").unwrap(),
            Some("Bathroom tap repair".into())
        );
        assert_eq!(clean_note(" ").unwrap(), None);
        assert!(clean_note(&"x".repeat(200)).is_ok());
        assert!(clean_note(&"x".repeat(201)).is_err());
        assert!(clean_note("two\nlines").is_err());
    }

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
            expires_at: now_unix() + 7 * 86_400,
            confs_required: 1,
            metadata_json: "{}".to_string(),
        }
    }

    #[tokio::test]
    async fn manual_invoices_are_kept_apart_from_shop_orders() {
        let db = Db::open(":memory:").unwrap();
        db.call(|conn| {
            let (s, _, _) = store::create(conn, "Sam's Plumbing", None)?;
            for id in ["shop1", "m1", "m2", "m3"] {
                insert(conn, &new_invoice(&s.id, id))?;
            }
            assert!(!is_enabled(conn)?, "off until the owner turns it on");
            set_enabled(conn, true)?;
            assert!(is_enabled(conn)?);

            finish_new(conn, &s.id, "m1", Some("Tap repair"), "1", "Sam Rivera")?;
            finish_new(conn, &s.id, "m2", None, "INV_7%", "Alex's Café")?;
            assert!(number_taken(conn, "inv_7%")?);
            assert!(!number_taken(conn, "2")?);

            // A number taken in the meantime: refused, and the invoice is gone.
            let err = finish_new(conn, &s.id, "m3", None, "1", "").unwrap_err();
            assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
            assert!(crate::invoice::get(conn, &s.id, "m3")?.is_none());

            assert!(is_manual(conn, "m1")? && !is_manual(conn, "shop1")?);
            let all = list(conn, &s.id, None, 50)?;
            assert_eq!(all.len(), 2, "shop orders are not on the Invoices tab");
            let m1 = get(conn, &s.id, "m1")?.unwrap();
            assert_eq!(m1.note.as_deref(), Some("Tap repair"));
            assert_eq!(m1.customer_name.as_deref(), Some("Sam Rivera"));
            assert!(get(conn, &s.id, "shop1")?.is_none());

            // Search by number or name; % and _ are plain characters.
            assert_eq!(list(conn, &s.id, Some("rivera"), 50)?.len(), 1);
            assert_eq!(list(conn, &s.id, Some("_7%"), 50)?.len(), 1);
            assert_eq!(list(conn, &s.id, Some("%"), 50)?.len(), 1);
            assert_eq!(list(conn, &s.id, Some("nobody"), 50)?.len(), 0);
            assert_eq!(export_all(conn, &s.id)?.len(), 2);
            Ok(())
        })
        .await
        .unwrap();
    }
}
