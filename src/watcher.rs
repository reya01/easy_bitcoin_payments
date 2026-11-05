//! Payment watcher: polls the wallet's payment history and drives the
//! invoice state machine; keeps the VTXO expiry queue in sync with the
//! wallet's live VTXO list.
//!
//! Polling today (barkd exposes no subscription we rely on); a subscription
//! adapter can be added later behind the same `tick` contract. The loop,
//! address matching, state transitions, and webhook emission are real and
//! unit-tested — the wallet source is injected through `WalletAdapter`
//! (barkd in production, fakes in tests).
//!
//! The history cursor is persisted in the `settings` table so a restart
//! loses nothing; re-delivered payments are deduped by the payments table's
//! unique VTXO/txid guards, so at-least-once polling is safe.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, Connection};

use crate::db::Db;
use crate::error::AppError;
use crate::invoice;
use crate::models::InvoiceStatus;
use crate::store;
use crate::util::{now_unix, to_rfc3339};
use crate::wallet::{WalletAdapter, WalletVtxo};
use crate::webhook::WebhookEmitter;

/// UNIX time of the last watcher tick that completed without error; 0 until
/// the first one. Surfaced by `/api/v1/health` so a wedged or crash-looping
/// watcher is visible to monitoring (beta hardening).
pub static LAST_TICK_UNIX: AtomicI64 = AtomicI64::new(0);

/// `settings` key holding the persisted history cursor (UNIX seconds).
const HISTORY_CURSOR_KEY: &str = "watcher.history_cursor";

pub struct PollingWatcher {
    wallet: Arc<dyn WalletAdapter>,
    db: Db,
    /// Unused since the durable outbox (webhook_outbox.rs) took over
    /// delivery; kept so `PollingWatcher::new`'s signature (used by many
    /// tests) stays stable.
    #[allow(dead_code)]
    webhooks: Arc<WebhookEmitter>,
    interval: Duration,
    cursor: Mutex<i64>,
    /// Renewal lead time in seconds; each expiry-queue row's `renew_by` is
    /// its per-VTXO expiry minus this.
    renewal_lead_secs: i64,
    /// Credit on-chain-rail payments? Set from `config.onchain_rail_enabled`
    /// via [`PollingWatcher::with_onchain_rail`]; `new()` defaults to true so
    /// existing tests keep their behaviour. When false, on-chain payments are
    /// parked in `unmatched_payments` instead of crediting an invoice.
    onchain_rail_enabled: bool,
}

impl PollingWatcher {
    pub fn new(
        wallet: Arc<dyn WalletAdapter>,
        db: Db,
        webhooks: Arc<WebhookEmitter>,
        interval: Duration,
        renewal_lead_secs: i64,
    ) -> Self {
        Self {
            wallet,
            db,
            webhooks,
            interval,
            cursor: Mutex::new(0),
            renewal_lead_secs,
            onchain_rail_enabled: true,
        }
    }

    /// Builder: enable/disable crediting of on-chain-rail payments.
    pub fn with_onchain_rail(mut self, enabled: bool) -> Self {
        self.onchain_rail_enabled = enabled;
        self
    }

    /// Run forever, polling on the configured interval.
    pub async fn run(self) {
        // Resume from the persisted cursor so a restart mid-payment loses
        // nothing (re-deliveries are deduped, never double-credited).
        match self
            .db
            .call(|conn| crate::db::get_setting(conn, HISTORY_CURSOR_KEY))
            .await
        {
            Ok(Some(raw)) => match raw.parse::<i64>() {
                Ok(resume_from) => {
                    if let Ok(mut cursor) = self.cursor.lock() {
                        *cursor = resume_from;
                    }
                    tracing::info!(
                        cursor = resume_from,
                        "watcher resumed from the persisted history cursor"
                    );
                }
                Err(_) => {
                    tracing::warn!("persisted watcher cursor was not an integer; starting from 0")
                }
            },
            Ok(None) => {}
            Err(err) => tracing::warn!(error = %err, "could not load the persisted watcher cursor"),
        }

        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // SUPERVISION (beta hardening): each tick runs in its own spawned
        // task, so a panic inside one tick is logged and the loop carries on
        // instead of silently ending payment detection until a restart.
        let this = Arc::new(self);
        loop {
            ticker.tick().await;
            let worker = Arc::clone(&this);
            match tokio::spawn(async move { worker.tick().await }).await {
                Ok(Ok(applied)) => {
                    LAST_TICK_UNIX.store(now_unix(), Ordering::Relaxed);
                    if applied > 0 {
                        tracing::info!(applied, "watcher applied payments");
                    }
                }
                Ok(Err(err)) => tracing::warn!(error = %err, "watcher tick failed"),
                Err(join_err) => {
                    tracing::error!(error = %join_err, "watcher tick PANICKED; continuing")
                }
            }
        }
    }

    /// One poll cycle. Returns the number of payments applied to invoices.
    pub async fn tick(&self) -> Result<usize, AppError> {
        let since = *self
            .cursor
            .lock()
            .map_err(|_| AppError::Internal("watcher cursor poisoned".into()))?;

        // BETA-HARDENING: the wallet adapter is synchronous and (for barkd)
        // blocks on HTTP. Run it on the blocking pool so a slow barkd cannot
        // stall a tokio worker thread.
        let wallet = Arc::clone(&self.wallet);
        let payments = tokio::task::spawn_blocking(move || wallet.poll_payments(since))
            .await
            .map_err(|e| AppError::Internal(format!("wallet poll task failed: {e}")))?
            .map_err(|e| AppError::Internal(format!("wallet poll failed: {e}")))?;

        let mut applied = 0usize;
        let mut newest = since;

        for payment in payments {
            newest = newest.max(payment.detected_at);

            // On-chain rail switched off (default): never auto-credit it. The
            // on-chain receive shape is unpinned, so we park the payment for
            // the merchant instead of risking a wrong credit.
            if matches!(payment.rail, crate::models::Rail::Onchain) && !self.onchain_rail_enabled {
                let (addr, rail) = (payment.address.clone(), payment.rail.as_str().to_string());
                let (vtxo, txid) = (payment.vtxo_id.clone(), payment.txid.clone());
                let (amount, seen) = (payment.amount_sats, payment.detected_at);
                let is_new = self
                    .db
                    .call(move |conn| {
                        invoice::record_unmatched(
                            conn,
                            &invoice::UnmatchedInput {
                                address: &addr,
                                rail: &rail,
                                amount_sats: amount,
                                vtxo_id: vtxo.as_deref(),
                                txid: txid.as_deref(),
                                detected_at: seen,
                                reason_override: Some("onchain_rail_disabled"),
                            },
                        )
                    })
                    .await?;
                if is_new {
                    tracing::warn!(
                        amount_sats = payment.amount_sats,
                        "on-chain payment seen while the on-chain rail is disabled; parked in unmatched_payments"
                    );
                }
                continue;
            }

            let address = payment.address.clone();
            let invoice = self
                .db
                .call(move |conn| invoice::find_pending_by_address(conn, &address))
                .await?;
            // R6: completed after the invoice's deadline (plus skew grace)?
            // Then it is a late payment even though `expire_due` has not
            // flipped the invoice yet; deterministic on the wallet's time.
            let mut late_reason: Option<&'static str> = None;
            let invoice = invoice.filter(|i| {
                if payment.detected_at > i.expires_at + invoice::LATE_PAYMENT_GRACE_SECS {
                    late_reason = Some("paid_after_expiry");
                    false
                } else {
                    true
                }
            });

            let Some(invoice) = invoice else {
                // BETA-HARDENING: no PENDING invoice owns this address. If it
                // belongs to an expired/closed invoice, the funds are in the
                // till and MUST be recorded (late payment) rather than
                // dropped. Unknown addresses (e.g. reserve funding) are
                // ignored by `record_unmatched`.
                let (addr, rail) = (payment.address.clone(), payment.rail.as_str().to_string());
                let (vtxo, txid) = (payment.vtxo_id.clone(), payment.txid.clone());
                let (amount, seen) = (payment.amount_sats, payment.detected_at);
                let is_new = self
                    .db
                    .call(move |conn| {
                        invoice::record_unmatched(
                            conn,
                            &invoice::UnmatchedInput {
                                address: &addr,
                                rail: &rail,
                                amount_sats: amount,
                                vtxo_id: vtxo.as_deref(),
                                txid: txid.as_deref(),
                                detected_at: seen,
                                reason_override: late_reason,
                            },
                        )
                    })
                    .await?;
                if is_new {
                    tracing::warn!(
                        amount_sats = payment.amount_sats,
                        "late/unmatched payment recorded in unmatched_payments; merchant must reconcile"
                    );
                }
                continue;
            };

            let store_id = invoice.store_id.clone();
            let observed = invoice::ObservedPayment {
                invoice_id: invoice.id.clone(),
                rail: payment.rail.as_str().to_string(),
                amount_sats: payment.amount_sats,
                vtxo_id: payment.vtxo_id.clone(),
                txid: payment.txid.clone(),
                detected_at: payment.detected_at,
                confirmed_at: payment.confirmed_at,
            };

            // A determinically-rejected payment (amount <= 0, or an illegal
            // transition) can never succeed: log it and move on so the
            // cursor advances past it. Aborting the tick here would freeze
            // the cursor on the same batch forever — a silent,
            // permanent payment-processing outage on one bad entry.
            let outcome = match self
                .db
                .call(move |conn| invoice::record_payment(conn, &observed))
                .await
            {
                Ok(outcome) => outcome,
                Err(err @ AppError::BadRequest(_)) | Err(err @ AppError::Conflict(_)) => {
                    tracing::warn!(error = %err, "payment rejected as permanently unrecordable; skipping it");
                    continue;
                }
                Err(err) => return Err(err),
            };

            let Some((updated, status)) = outcome else {
                continue;
            };

            applied += 1;
            tracing::info!(
                invoice_id = %updated.id,
                status = %status,
                amount_sats = updated.paid_sats,
                "payment applied"
            );

            self.emit_invoice_event(&store_id, &updated, status).await;
        }

        // Advance and persist the cursor. Persisting after applying means a
        // crash mid-tick re-delivers payments; the unique VTXO/txid guards
        // turn that into a no-op instead of a double credit.
        //
        // Advance ONLY to the newest wallet-reported timestamp — never to
        // local now(): local clock and wallet timestamps are different
        // clocks, and a capped/truncated page must never let the cursor
        // jump past entries it never saw.
        let next_cursor = newest;
        if next_cursor > since {
            if let Ok(mut cursor) = self.cursor.lock() {
                *cursor = next_cursor;
            }
            let value = next_cursor.to_string();
            self.db
                .call(move |conn| crate::db::set_setting(conn, HISTORY_CURSOR_KEY, &value))
                .await?;
        }

        // Keep the VTXO expiry queue in sync with the wallet's live list.
        // Every VTXO carries its own expiry: board/refresh VTXOs live ~28
        // days, Lightning-received VTXOs only 1-3 days (learn_lifetime.md).
        let wallet = Arc::clone(&self.wallet);
        let vtxo_result = tokio::task::spawn_blocking(move || wallet.vtxos())
            .await
            .map_err(|e| AppError::Internal(format!("wallet vtxos task failed: {e}")))?;
        match vtxo_result {
            Ok(vtxos) => {
                let lead = self.renewal_lead_secs;
                self.db
                    .call(move |conn| upsert_vtxo_expiry(conn, &vtxos, lead))
                    .await?;
            }
            Err(err) => tracing::warn!(error = %err, "could not refresh the vtxo expiry queue"),
        }

        // Expire invoices whose rate lock has lapsed even when no checkout
        // page is open to notice, so stores receive `invoice.expired` and
        // `GET /api/v1/invoices/{id}` stops reporting `pending` for dead
        // invoices.
        let expired = self.db.call(invoice::expire_due).await?;
        for invoice in expired {
            tracing::info!(invoice_id = %invoice.id, "invoice expired");
            self.emit_invoice_event(&invoice.store_id, &invoice, InvoiceStatus::Expired)
                .await;
        }

        Ok(applied)
    }

    async fn emit_invoice_event(
        &self,
        store_id: &str,
        invoice: &crate::models::Invoice,
        status: InvoiceStatus,
    ) {
        let store_id = store_id.to_string();
        let store = match self.db.call(move |conn| store::get(conn, &store_id)).await {
            Ok(Some(store)) => store,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!(error = %err, "could not load store for webhook");
                return;
            }
        };

        // No webhook configured for this store: nothing to queue.
        if store.webhook_url.is_none() || store.webhook_secret.is_none() {
            return;
        }

        let event = match status {
            InvoiceStatus::Paid => "invoice.paid",
            InvoiceStatus::Overpaid => "invoice.overpaid",
            InvoiceStatus::Underpaid => "invoice.underpaid",
            InvoiceStatus::Expired => "invoice.expired",
            InvoiceStatus::Cancelled | InvoiceStatus::Pending => return,
        };

        let payload = serde_json::json!({
            "event": event,
            "invoice_id": invoice.id,
            "store_id": invoice.store_id,
            "order_id": invoice.order_id,
            "status": invoice.status.as_str(),
            "amount_sats": invoice.amount_sats,
            "paid_sats": invoice.paid_sats,
            "paid_at": invoice.paid_at.map(to_rfc3339),
        });

        let idempotency_key = format!("{}:{}:{}", invoice.id, event, invoice.paid_sats);

        // BETA-HARDENING: write the event to the durable outbox
        // (`webhook_deliveries`, migration 0006). The dispatcher in
        // `webhook_outbox::run` delivers it with backoff and survives
        // restarts. The idempotency key is UNIQUE, so re-observing the same
        // payment cannot queue the event twice.
        let (store_id, invoice_id) = (invoice.store_id.clone(), invoice.id.clone());
        let event_name = event.to_string();
        if let Err(err) = self
            .db
            .call(move |conn| {
                crate::webhook_outbox::enqueue(
                    conn,
                    &store_id,
                    &invoice_id,
                    &event_name,
                    &idempotency_key,
                    &payload,
                )
            })
            .await
        {
            tracing::error!(error = %err, event, "could not queue webhook event");
        }
    }
}

/// Rebuild the expiry queue from the wallet's live VTXO list: upsert every
/// reported VTXO with its real per-VTXO expiry, and mark rows the wallet no
/// longer reports as `gone` (spent, swept, or exited).
fn upsert_vtxo_expiry(
    conn: &Connection,
    vtxos: &[WalletVtxo],
    renewal_lead_secs: i64,
) -> Result<(), AppError> {
    let now = now_unix();

    let mut stmt = conn.prepare("SELECT id FROM vtxo_expiry WHERE status = 'live'")?;
    let live_ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    let present: HashSet<&str> = vtxos.iter().map(|v| v.id.as_str()).collect();
    for id in &live_ids {
        if !present.contains(id.as_str()) {
            conn.execute(
                "UPDATE vtxo_expiry SET status = 'gone', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
        }
    }

    for v in vtxos {
        // Unknown expiries (0) get renew_by = now: the queue treats them as
        // urgent rather than guessing a lifetime.
        let renew_by = if v.expires_at > 0 {
            v.expires_at - renewal_lead_secs
        } else {
            now
        };
        conn.execute(
            "INSERT INTO vtxo_expiry (id, amount_sats, expiry_at, renew_by, likely_lightning, status, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'live', ?6)
             ON CONFLICT(id) DO UPDATE SET
                amount_sats = excluded.amount_sats,
                expiry_at = excluded.expiry_at,
                renew_by = excluded.renew_by,
                likely_lightning = excluded.likely_lightning,
                status = 'live',
                updated_at = excluded.updated_at",
            params![v.id, v.amount_sats, v.expires_at, renew_by, v.likely_lightning, now],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::NewInvoice;
    use crate::models::Rail;
    use crate::wallet::WalletPayment;
    use crate::wallet::{AspStatus, ReceiveAddresses, WalletError, WalletHealth};

    struct FakeWallet {
        payments: Vec<WalletPayment>,
        vtxos: Vec<WalletVtxo>,
    }

    impl WalletAdapter for FakeWallet {
        fn receive_addresses(&self, invoice_id: &str) -> Result<ReceiveAddresses, WalletError> {
            Ok(ReceiveAddresses {
                ark_address: format!("ark1q{invoice_id}"),
                onchain_address: format!("bc1q{invoice_id}"),
            })
        }

        fn balance_sats(&self) -> Result<i64, WalletError> {
            Ok(0)
        }

        fn poll_payments(&self, _since: i64) -> Result<Vec<WalletPayment>, WalletError> {
            Ok(self.payments.clone())
        }

        fn health(&self) -> WalletHealth {
            WalletHealth {
                reachable: true,
                balance_sats: Some(0),
                asp: AspStatus::Ok,
                detail: "fake".into(),
            }
        }

        fn vtxos(&self) -> Result<Vec<WalletVtxo>, WalletError> {
            Ok(self.vtxos.clone())
        }
    }

    async fn setup(_payments: Vec<WalletPayment>) -> (Db, String) {
        let db = Db::open(":memory:").unwrap();
        let (store, _, _) = db
            .call(|conn| store::create(conn, "Shop", None))
            .await
            .unwrap();

        let store_id = store.id.clone();
        db.call(move |conn| {
            invoice::insert(
                conn,
                &NewInvoice {
                    id: "inv_1".to_string(),
                    store_id,
                    order_id: Some("wc-1".to_string()),
                    amount_sats: 1000,
                    fiat_currency: None,
                    fiat_amount: None,
                    rate: None,
                    rate_source: None,
                    rate_locked_at: None,
                    ark_address: "ark1qinv_1".to_string(),
                    onchain_address: "bc1qinv_1".to_string(),
                    expires_at: now_unix() + 1200,
                    confs_required: 1,
                    metadata_json: "{}".to_string(),
                },
            )
        })
        .await
        .unwrap();

        (db, store.id)
    }

    #[tokio::test]
    async fn tick_applies_matching_payment() {
        let payment = WalletPayment {
            address: "ark1qinv_1".to_string(),
            rail: Rail::Ark,
            amount_sats: 1000,
            vtxo_id: Some("vtxo_1".to_string()),
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        };
        let (db, store_id) = setup(vec![payment]).await;

        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![WalletPayment {
                    address: "ark1qinv_1".to_string(),
                    rail: Rail::Ark,
                    amount_sats: 1000,
                    vtxo_id: Some("vtxo_1".to_string()),
                    txid: None,
                    detected_at: now_unix(),
                    confirmed_at: None,
                }],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );

        let applied = watcher.tick().await.unwrap();
        assert_eq!(applied, 1);

        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);
        assert_eq!(invoice.paid_sats, 1000);
    }

    fn onchain_payment() -> WalletPayment {
        WalletPayment {
            address: "bc1qinv_1".to_string(),
            rail: Rail::Onchain,
            amount_sats: 1000,
            vtxo_id: None,
            txid: Some("tx_onchain_1".to_string()),
            detected_at: now_unix(),
            confirmed_at: Some(now_unix()),
        }
    }

    #[tokio::test]
    async fn onchain_payment_is_parked_not_credited_when_the_rail_is_disabled() {
        let (db, store_id) = setup(vec![]).await;
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![onchain_payment()],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        )
        .with_onchain_rail(false);

        assert_eq!(watcher.tick().await.unwrap(), 0, "must not credit");

        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Pending);

        let parked: i64 = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM unmatched_payments WHERE reason = 'onchain_rail_disabled'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(parked, 1);
    }

    #[tokio::test]
    async fn onchain_payment_is_credited_when_the_rail_is_enabled() {
        let (db, store_id) = setup(vec![]).await;
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![onchain_payment()],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        )
        .with_onchain_rail(true);

        assert_eq!(watcher.tick().await.unwrap(), 1);
        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn repeated_ticks_do_not_report_a_credited_payment_as_unmatched() {
        // The wallet keeps returning the same history entry on every poll.
        let (db, _) = setup(vec![]).await;
        let payment = WalletPayment {
            address: "ark1qinv_1".to_string(),
            rail: Rail::Ark,
            amount_sats: 1000,
            vtxo_id: Some("vtxo_1".to_string()),
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        };
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![payment],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        assert_eq!(watcher.tick().await.unwrap(), 0);

        let unmatched: i64 = db
            .call(|conn| {
                Ok(conn.query_row("SELECT COUNT(*) FROM unmatched_payments", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(unmatched, 0, "credited payments are never 'late'");
    }

    #[tokio::test]
    async fn payment_completed_after_the_deadline_is_parked_even_if_still_pending() {
        // R6: the invoice has not been flipped to expired yet (same tick),
        // but the wallet says the payment completed long after expires_at.
        let (db, _) = setup(vec![]).await;
        let payment = WalletPayment {
            address: "ark1qinv_1".to_string(),
            rail: Rail::Ark,
            amount_sats: 1000,
            vtxo_id: Some("vtxo_late".to_string()),
            txid: None,
            detected_at: now_unix() + 30 * 86_400,
            confirmed_at: None,
        };
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![payment],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        assert_eq!(watcher.tick().await.unwrap(), 0, "must not be credited");
        let reason: String = db
            .call(|conn| {
                Ok(conn.query_row("SELECT reason FROM unmatched_payments", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(reason, "paid_after_expiry");
    }

    #[tokio::test]
    async fn tick_ignores_unknown_addresses() {
        let (db, _) = setup(vec![]).await;
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![WalletPayment {
                    address: "ark1qsomeone_else".to_string(),
                    rail: Rail::Ark,
                    amount_sats: 5000,
                    vtxo_id: Some("vtxo_x".to_string()),
                    txid: None,
                    detected_at: now_unix(),
                    confirmed_at: None,
                }],
                vtxos: vec![],
            }),
            db,
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );

        assert_eq!(watcher.tick().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn tick_is_idempotent_for_repeated_polls() {
        let payment = WalletPayment {
            address: "ark1qinv_1".to_string(),
            rail: Rail::Ark,
            amount_sats: 1000,
            vtxo_id: Some("vtxo_1".to_string()),
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        };
        let (db, _) = setup(vec![]).await;

        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![payment.clone()],
                vtxos: vec![],
            }),
            db,
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );

        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(watcher.tick().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn tick_persists_the_history_cursor() {
        let (db, _) = setup(vec![]).await;
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![WalletPayment {
                    address: "ark1qinv_1".to_string(),
                    rail: Rail::Ark,
                    amount_sats: 1000,
                    vtxo_id: Some("vtxo_1".to_string()),
                    txid: None,
                    detected_at: now_unix(),
                    confirmed_at: None,
                }],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        watcher.tick().await.unwrap();

        let saved = db
            .call(|conn| crate::db::get_setting(conn, HISTORY_CURSOR_KEY))
            .await
            .unwrap()
            .expect("cursor persisted");
        assert_eq!(
            saved.parse::<i64>().unwrap(),
            *watcher.cursor.lock().unwrap()
        );
    }

    #[tokio::test]
    async fn tick_builds_and_prunes_the_vtxo_expiry_queue() {
        let (db, _) = setup(vec![]).await;
        let now = now_unix();
        let v1 = WalletVtxo {
            id: "v1".into(),
            amount_sats: 50_000,
            expires_at: now + 28 * 86_400,
            likely_lightning: false,
        };
        let v2 = WalletVtxo {
            id: "v2".into(),
            amount_sats: 2_100,
            expires_at: now + 2 * 86_400,
            likely_lightning: true,
        };

        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![],
                vtxos: vec![v1.clone(), v2.clone()],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        watcher.tick().await.unwrap();

        let rows: Vec<(String, i64, i64, String)> = db
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, renew_by, likely_lightning, status FROM vtxo_expiry ORDER BY id",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);

        // The Lightning-received VTXO (1-3 day lifetime) is flagged and its
        // renew_by is already past: urgent.
        let (id, renew_by, likely_ln, status) = rows.last().unwrap().clone();
        assert_eq!(id, "v2");
        assert_eq!(likely_ln, 1);
        assert_eq!(status, "live");
        assert!(renew_by < now);
        // The 28-day VTXO renews three weeks out.
        assert!(rows[0].1 > now);

        // A VTXO the wallet no longer reports leaves the live queue.
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![],
                vtxos: vec![v2],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        watcher.tick().await.unwrap();
        let v1_status: String = db
            .call(|conn| {
                Ok(
                    conn.query_row("SELECT status FROM vtxo_expiry WHERE id = 'v1'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(v1_status, "gone");
    }
}
