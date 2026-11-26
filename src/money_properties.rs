//! Property tests for the money rules: thousands of random cases instead of a
//! few hand-picked ones. No extra crates: a small fixed-seed generator makes
//! every run the same, so a failure always reproduces (the message prints the
//! case number and seed).
//!
//! What is checked:
//! - `fiat_to_sats` stays in range, rounds to the nearest sat, and never gives
//!   fewer sats for more money (or for a lower price);
//! - `status_after_payment` agrees with plain arithmetic;
//! - `late_full_payment_ok` never revives on-chain money, part payments, or
//!   anything after the late window;
//! - `sats_with_usd` always shows the exact sats amount;
//! - `record_payment`, driven with random sequences of payments, repeats,
//!   expiries and cancellations, always matches a simple model of the rules:
//!   paid total = sum of recorded payments, nothing is counted twice, closed
//!   invoices take nothing except a full late Ark/Lightning payment, and the
//!   status follows the total.

use crate::db::Db;
use crate::invoice::{self, NewInvoice, ObservedPayment, LATE_FULL_PAYMENT_SECS, MAX_INVOICE_SATS};
use crate::models::InvoiceStatus;
use crate::rates::{fiat_to_sats, sats_with_usd};
use crate::store;
use crate::util::now_unix;
use rusqlite::{params, Connection};

/// xorshift64*: tiny, fast, and the same on every machine.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi - lo) as u64 + 1;
        lo + (self.next_u64() % span) as i64
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next_u64() % xs.len() as u64) as usize]
    }
    /// A float spread over many orders of magnitude, in `[10^lo, 10^hi)`.
    fn log_uniform(&mut self, lo: f64, hi: f64) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        10f64.powf(lo + u * (hi - lo))
    }
}

const CASES: usize = 20_000;

#[test]
fn fiat_to_sats_is_in_range_rounded_and_never_goes_backwards() {
    let mut rng = Rng::new(0xEB9_0001);
    for case in 0..CASES {
        let price = rng.log_uniform(2.0, 8.0); // 100 to 100,000,000 per bitcoin
        let a = (rng.log_uniform(-2.0, 7.0) * 100.0).round() / 100.0; // cents
        let b = a + (rng.range(0, 100_000) as f64) / 100.0;
        let exact = a / price * 1e8;
        match fiat_to_sats(a, price) {
            Ok(s) => {
                assert!(
                    (1..=MAX_INVOICE_SATS).contains(&s),
                    "case {case}: {a} at {price} gave {s}"
                );
                assert!(
                    (s as f64 - exact).abs() <= 0.5 + exact * 1e-12,
                    "case {case}: {a} at {price} gave {s}, exact {exact}"
                );
                let more = fiat_to_sats(b, price).expect("a larger amount is still in range");
                assert!(more >= s, "case {case}: {b} gave {more} < {s} for {a}");
                // A higher price never gives MORE sats for the same money.
                if let Ok(cheaper) = fiat_to_sats(a, price * 1.5) {
                    assert!(cheaper <= s, "case {case}: higher price gave more sats");
                }
            }
            Err(_) => assert!(
                exact < 1.0 || exact > MAX_INVOICE_SATS as f64,
                "case {case}: {a} at {price} (exact {exact} sats) was refused"
            ),
        }
    }
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            fiat_to_sats(bad, 60_000.0).is_err(),
            "amount {bad} must be refused"
        );
        assert!(
            fiat_to_sats(10.0, bad).is_err(),
            "price {bad} must be refused"
        );
    }
}

#[test]
fn status_follows_the_arithmetic() {
    let mut rng = Rng::new(0xEB9_0002);
    for case in 0..CASES {
        let due = rng.range(1, MAX_INVOICE_SATS);
        let paid = match rng.range(0, 3) {
            0 => due,
            1 => due - rng.range(0, due.min(1_000_000)),
            2 => due.saturating_add(rng.range(1, 1_000_000)),
            _ => rng.range(0, MAX_INVOICE_SATS),
        };
        let want = if paid > due {
            InvoiceStatus::Overpaid
        } else if paid == due {
            InvoiceStatus::Paid
        } else {
            InvoiceStatus::Underpaid
        };
        assert_eq!(
            invoice::status_after_payment(paid, due),
            want,
            "case {case}: {paid}/{due}"
        );
    }
}

fn sample_invoice(rng: &mut Rng) -> crate::models::Invoice {
    let amount = rng.range(1, 10_000_000);
    let paid = if rng.range(0, 1) == 0 {
        0
    } else {
        rng.range(0, amount)
    };
    crate::models::Invoice {
        id: "inv".into(),
        store_id: "s".into(),
        order_id: None,
        status: InvoiceStatus::Expired,
        amount_sats: amount,
        fiat_currency: None,
        fiat_amount: None,
        rate: None,
        rate_source: None,
        rate_locked_at: None,
        ark_address: "ark1qexample".into(),
        expires_at: 1_700_000_000,
        paid_at: None,
        paid_sats: paid,
        confs_required: 1,
        metadata_json: "{}".into(),
        created_at: 1_699_998_800,
    }
}

#[test]
fn late_payments_revive_only_full_instant_payments_inside_the_window() {
    let mut rng = Rng::new(0xEB9_0003);
    for case in 0..CASES {
        let inv = sample_invoice(&mut rng);
        let rail = *rng.pick(&["ark", "lightning", "onchain"]);
        let credited = rng.range(1, inv.amount_sats * 2);
        let detected = inv.expires_at + rng.range(-3600, 2 * LATE_FULL_PAYMENT_SECS);
        let ok = invoice::late_full_payment_ok(&inv, rail, credited, detected);
        let want = rail != "onchain"
            && detected <= inv.expires_at + LATE_FULL_PAYMENT_SECS
            && inv.paid_sats + credited >= inv.amount_sats;
        assert_eq!(
            ok, want,
            "case {case}: {rail} {credited} at {detected} on {inv:?}"
        );
    }
}

#[test]
fn the_shown_sats_amount_is_always_exact() {
    let mut rng = Rng::new(0xEB9_0004);
    for case in 0..CASES {
        let sats = match rng.range(0, 2) {
            0 => rng.range(0, 999),
            1 => rng.range(0, 10_000_000),
            _ => rng.range(-MAX_INVOICE_SATS, MAX_INVOICE_SATS),
        };
        let price = match rng.range(0, 2) {
            0 => None,
            1 => Some(rng.log_uniform(2.0, 7.0)),
            _ => Some(*rng.pick(&[0.0, -5.0, f64::NAN, f64::INFINITY])),
        };
        let shown = sats_with_usd(sats, price);
        let number: String = shown.split(" sats").next().unwrap().replace(',', "");
        assert_eq!(
            number.parse::<i64>().ok(),
            Some(sats),
            "case {case}: {shown:?}"
        );
        let groups: Vec<&str> = shown
            .split(" sats")
            .next()
            .unwrap()
            .trim_start_matches('-')
            .split(',')
            .collect();
        assert!(
            groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3),
            "case {case}: {shown:?}"
        );
    }
}

/// What the rules say should happen, kept as plainly as possible.
#[derive(Clone, Debug)]
struct Model {
    status: InvoiceStatus,
    paid: i64,
    amount: i64,
    expires_at: i64,
}

fn read(conn: &Connection, id: &str) -> (InvoiceStatus, i64, i64, i64) {
    let inv = conn
        .query_row(
            "SELECT * FROM invoices WHERE id = ?1",
            params![id],
            invoice::row_to_invoice,
        )
        .unwrap();
    let (sum, n): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(amount_sats), 0), COUNT(*) FROM payments WHERE invoice_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    (inv.status, inv.paid_sats, sum, n)
}

#[tokio::test]
async fn record_payment_always_matches_the_rules() {
    let db = Db::open(":memory:").unwrap();
    db.call(|conn| {
        let (s, _, _) = store::create(conn, "Property Shop", None)?;
        let mut rng = Rng::new(0xEB9_0005);
        let mut seen: Vec<String> = Vec::new(); // vtxo ids already recorded anywhere
        let mut next_key = 0u64;
        for run in 0..400 {
            let id = format!("inv_{run}");
            let amount = rng.range(1, 5_000_000);
            let now = now_unix();
            invoice::insert(
                conn,
                &NewInvoice {
                    id: id.clone(),
                    store_id: s.id.clone(),
                    order_id: None,
                    amount_sats: amount,
                    fiat_currency: None,
                    fiat_amount: None,
                    rate: None,
                    rate_source: None,
                    rate_locked_at: None,
                    ark_address: format!("ark1qprop{run}"),
                    expires_at: now + 1200,
                    confs_required: 1,
                    metadata_json: "{}".into(),
                },
            )?;
            let mut m = Model { status: InvoiceStatus::Pending, paid: 0, amount, expires_at: now + 1200 };

            for step in 0..rng.range(1, 12) {
                let ctx = format!("run {run} step {step}");
                match rng.range(0, 9) {
                    // Expire it (its time runs out).
                    0 => {
                        conn.execute("UPDATE invoices SET expires_at = ?1 WHERE id = ?2", params![now - 10, id])?;
                        m.expires_at = now - 10;
                        invoice::expire_if_due(conn, &id)?;
                        if matches!(m.status, InvoiceStatus::Pending | InvoiceStatus::Underpaid) {
                            m.status = InvoiceStatus::Expired;
                        }
                    }
                    // The owner cancels it.
                    1 => {
                        let r = invoice::cancel(conn, &s.id, &id);
                        if m.status == InvoiceStatus::Pending {
                            assert!(r.is_ok(), "{ctx}: cancelling a pending invoice works");
                            m.status = InvoiceStatus::Cancelled;
                        } else {
                            assert!(r.is_err(), "{ctx}: only a pending invoice can be cancelled");
                        }
                    }
                    // A payment arrives (sometimes a repeat of one already recorded).
                    _ => {
                        let rail = *rng.pick(&["ark", "lightning", "onchain"]);
                        let repeat = !seen.is_empty() && rng.range(0, 4) == 0;
                        let key = if repeat {
                            rng.pick(&seen).clone()
                        } else {
                            next_key += 1;
                            format!("vtxo_{next_key}")
                        };
                        let credited = match rng.range(0, 3) {
                            0 => m.amount - m.paid,
                            1 => rng.range(1, m.amount.max(1)),
                            2 => m.amount - m.paid + rng.range(1, 1000),
                            _ => rng.range(1, MAX_INVOICE_SATS / 4),
                        }
                        .max(1);
                        let detected = m.expires_at + rng.range(-600, 2 * LATE_FULL_PAYMENT_SECS);
                        let before = read(conn, &id);
                        let got = invoice::record_payment(
                            conn,
                            &ObservedPayment {
                                invoice_id: id.clone(),
                                rail: rail.into(),
                                amount_sats: credited,
                                received_sats: None,
                                vtxo_id: Some(key.clone()),
                                txid: None,
                                detected_at: detected,
                                confirmed_at: None,
                            },
                        );

                        let duplicate = seen.contains(&key);
                        let open = matches!(m.status, InvoiceStatus::Pending | InvoiceStatus::Underpaid);
                        let revive = m.status == InvoiceStatus::Expired
                            && rail != "onchain"
                            && detected <= m.expires_at + LATE_FULL_PAYMENT_SECS
                            && m.paid + credited >= m.amount;
                        let after = read(conn, &id);
                        if duplicate || !(open || revive) {
                            assert!(matches!(got, Ok(None)), "{ctx}: must be refused quietly, got {got:?}");
                            assert_eq!(after, before, "{ctx}: nothing may change");
                        } else {
                            let total = m.paid + credited;
                            let next = invoice::status_after_payment(total, m.amount);
                            let (inv, st) = got
                                .unwrap_or_else(|e| panic!("{ctx}: payment must be recorded: {e:?}"))
                                .unwrap_or_else(|| panic!("{ctx}: payment must be recorded"));
                            assert_eq!((st, inv.paid_sats), (next, total), "{ctx}");
                            m.paid = total;
                            m.status = next;
                            seen.push(key);
                        }
                    }
                }
                // The books always add up.
                let (status, paid, sum, _) = read(conn, &id);
                assert_eq!(status, m.status, "run {run} step {step}: status");
                assert_eq!(paid, m.paid, "run {run} step {step}: paid total");
                assert_eq!(paid, sum, "run {run} step {step}: paid total = sum of payments");
            }
        }
        // No payment is recorded twice anywhere.
        let dups: i64 = conn.query_row(
            "SELECT COUNT(*) FROM (SELECT vtxo_id FROM payments GROUP BY vtxo_id HAVING COUNT(*) > 1)",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(dups, 0, "a payment was recorded twice");
        Ok(())
    })
    .await
    .unwrap();
}
