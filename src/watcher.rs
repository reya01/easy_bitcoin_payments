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
use crate::wallet::{WalletAdapter, WalletPayment, WalletVtxo};
use crate::webhook::WebhookEmitter;

/// UNIX time of the last watcher tick that completed without error; 0 until
/// the first one. Surfaced by `/api/v1/health` so a wedged or crash-looping
/// watcher is visible to monitoring (beta hardening).
pub static LAST_TICK_UNIX: AtomicI64 = AtomicI64::new(0);

/// Did the late payment just written for `address` belong to an invoice made
/// on the Invoices tab?
fn late_payment_is_for_manual_invoice(
    conn: &rusqlite::Connection,
    address: &str,
    amount_sats: i64,
    detected_at: i64,
) -> Result<bool, AppError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM unmatched_payments u
                         JOIN invoices i ON i.id = u.invoice_id
                        WHERE u.address = ?1 AND u.amount_sats = ?2
                          AND u.detected_at = ?3 AND i.source = 'manual')",
        rusqlite::params![address, amount_sats, detected_at],
        |r| r.get(0),
    )?)
}

/// How long a remembered-as-handled wallet entry is trusted before the
/// watcher forgets everything and reads the whole history again. The database
/// guards are the real protection; this is only the safety net for the
/// shortcut below.
const HANDLED_FULL_PASS_SECS: i64 = 3_600;

/// Upper bound on remembered entries (about 100 bytes each). Past it the set is
/// emptied and refilled by the next pass.
const HANDLED_MAX: usize = 50_000;

/// `settings` key holding the persisted history cursor (UNIX seconds).
const HISTORY_CURSOR_KEY: &str = "watcher.history_cursor";

/// When the watcher first failed to save a payment it could see in the wallet
/// (unix seconds; empty or 0 = no trouble). Cleared by the next clean tick.
pub const SAVE_TROUBLE_KEY: &str = "payment_save_trouble_at";
/// Set once the phone alert for the current trouble has been queued, so a long
/// outage sends one alert, not one per retry.
const SAVE_TROUBLE_ALERTED_KEY: &str = "payment_save_trouble_alerted";
/// The latest failure (unix seconds), refreshed on every failure, so Home keeps
/// showing the line for as long as the trouble lasts, not 24 hours from its start.
const SAVE_TROUBLE_SEEN_KEY: &str = "payment_save_trouble_seen";
/// A short hiccup (a busy moment, a restart) is not worth a phone alert.
pub const SAVE_TROUBLE_ALERT_AFTER_SECS: i64 = 300;
/// Home stops showing the line after this long without a fresh failure, in
/// case the watcher itself stopped and can no longer clear it.
const SAVE_TROUBLE_SHOW_SECS: i64 = 86_400;

fn save_trouble_at(conn: &Connection) -> i64 {
    crate::settings::get(conn, SAVE_TROUBLE_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
}

/// A payment the wallet shows could not be saved. Remember since when, and
/// queue ONE phone alert once it has lasted [`SAVE_TROUBLE_ALERT_AFTER_SECS`].
/// The payment is not lost: the watcher tries it again every tick.
pub fn note_save_trouble(conn: &Connection, now: i64) -> Result<(), AppError> {
    let mut since = save_trouble_at(conn);
    if since <= 0 {
        since = now;
        crate::settings::set(conn, SAVE_TROUBLE_KEY, &now.to_string())?;
        crate::settings::set(conn, SAVE_TROUBLE_ALERTED_KEY, "")?;
    }
    crate::settings::set(conn, SAVE_TROUBLE_SEEN_KEY, &now.to_string())?;
    let alerted = crate::settings::get(conn, SAVE_TROUBLE_ALERTED_KEY)?
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if !alerted && now - since >= SAVE_TROUBLE_ALERT_AFTER_SECS {
        crate::alerts::queue_notice(conn, "payment_not_saved")?;
        crate::settings::set(conn, SAVE_TROUBLE_ALERTED_KEY, "1")?;
    }
    Ok(())
}

/// Every payment the wallet showed was saved: the trouble is over.
pub fn clear_save_trouble(conn: &Connection) -> Result<(), AppError> {
    if save_trouble_at(conn) > 0 {
        crate::settings::set(conn, SAVE_TROUBLE_KEY, "")?;
        crate::settings::set(conn, SAVE_TROUBLE_ALERTED_KEY, "")?;
        crate::settings::set(conn, SAVE_TROUBLE_SEEN_KEY, "")?;
    }
    Ok(())
}

/// Home should show the red "a payment could not be saved yet" line.
pub fn save_trouble_showing(conn: &Connection, now: i64) -> bool {
    let at = save_trouble_at(conn);
    let seen = crate::settings::get(conn, SAVE_TROUBLE_SEEN_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(at);
    at > 0 && now - seen < SAVE_TROUBLE_SHOW_SECS
}

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
    /// Wallet history entries the database already holds (in `payments` or
    /// `unmatched_payments`), with the time the set was last emptied. The
    /// wallet sends its whole history every tick; these are skipped instead of
    /// being looked up again. Only entries found in the database are added, so
    /// a failed write is retried on the next tick. In memory only: a restart
    /// does one full pass.
    handled: Mutex<(HashSet<String>, i64)>,
}

/// What identifies one wallet history entry for the "already handled" set. Two
/// entries that differ in anything that could matter (address, amount, rail,
/// confirmed or not) get different keys, so a changed entry is looked at again.
/// An entry with neither a VTXO id nor a txid is never skipped.
fn handled_key(p: &WalletPayment) -> Option<String> {
    if p.vtxo_id.is_none() && p.txid.is_none() {
        return None;
    }
    Some(format!(
        "{}|{}|{}|{}|{}|{}",
        p.rail.as_str(),
        p.address,
        p.amount_sats,
        p.vtxo_id.as_deref().unwrap_or(""),
        p.txid.as_deref().unwrap_or(""),
        p.confirmed_at.is_some()
    ))
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
            handled: Mutex::new((HashSet::new(), 0)),
        }
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
        let mut payments = tokio::task::spawn_blocking(move || wallet.poll_payments(since))
            .await
            .map_err(|e| AppError::Internal(format!("wallet poll task failed: {e}")))?
            .map_err(|e| AppError::Internal(format!("wallet poll failed: {e}")))?;

        let mut applied = 0usize;
        let mut newest = since;
        // The cursor follows everything the wallet reported, skipped or not.
        for p in &payments {
            newest = newest.max(p.detected_at);
        }

        // Skip entries the database already holds. Everything is looked at
        // again once an hour, and after every restart.
        {
            let mut guard = self.handled.lock().unwrap_or_else(|e| e.into_inner());
            let now = now_unix();
            if now - guard.1 >= HANDLED_FULL_PASS_SECS || guard.0.len() > HANDLED_MAX {
                guard.0.clear();
                guard.1 = now;
            }
            if !guard.0.is_empty() {
                payments.retain(|p| match handled_key(p) {
                    Some(k) => !guard.0.contains(&k),
                    None => true,
                });
            }
        }
        let looked_at = payments.clone();

        // One payment that cannot be saved must not hold up the others (a
        // persistent error on one row would otherwise stall every customer's
        // payment and every expiry). Each is handled on its own; the first
        // error is kept and returned at the end, and the cursor does not
        // advance while any payment is unsaved, so it is tried again.
        let mut first_error: Option<AppError> = None;
        for payment in payments {
            newest = newest.max(payment.detected_at);
            match self.handle_payment(payment).await {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "a payment could not be saved; carrying on with the others");
                    let now = now_unix();
                    if let Err(note_err) =
                        self.db.call(move |conn| note_save_trouble(conn, now)).await
                    {
                        tracing::warn!(error = %note_err, "could not note the payment-save trouble");
                    }
                    first_error.get_or_insert(err);
                }
            }
        }

        // Remember the entries that the database now provably holds. An entry
        // that was ignored or whose write failed is not remembered, so it is
        // looked at again next tick.
        for p in looked_at {
            let Some(key) = handled_key(&p) else { continue };
            let (vtxo, txid) = (p.vtxo_id.clone(), p.txid.clone());
            let held = self
                .db
                .call(move |conn| invoice::already_recorded(conn, vtxo.as_deref(), txid.as_deref()))
                .await?;
            if held {
                self.handled
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .0
                    .insert(key);
            }
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
        if first_error.is_none() && next_cursor > since {
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
            self.emit_invoice_event(&invoice.store_id, &invoice, InvoiceStatus::Expired, false)
                .await;
        }

        // Fill any gap in the shop's events (idempotent: an event already
        // queued is not queued again). Two days is far inside how long
        // delivered events are kept, so nothing is ever sent twice.
        let recent = self
            .db
            .call(|conn| invoice::recently_finished(conn, 2 * 86_400, 200))
            .await?;
        for invoice in recent {
            let store_id = invoice.store_id.clone();
            self.emit_invoice_event(&store_id, &invoice, invoice.status, false)
                .await;
        }

        // A payment that could not be saved: the rest of this check is done,
        // and the error is reported (the cursor did not move, so it is retried).
        if let Some(err) = first_error {
            return Err(err);
        }

        // Every payment the wallet showed is now saved (or parked).
        self.db.call(|conn| clear_save_trouble(conn)).await?;

        Ok(applied)
    }

    /// Handle one payment the wallet showed. `Ok(true)` = credited to an
    /// order, `Ok(false)` = nothing to credit (parked, ignored or already
    /// held). An error means THIS payment could not be saved right now; the
    /// caller carries on with the others and tries this one again next tick.
    async fn handle_payment(&self, payment: WalletPayment) -> Result<bool, AppError> {
        // EBP takes no regular (on-chain) Bitcoin payments. Money that still
        // arrives that way (for example sent to an old invoice address) is
        // never credited to an order: it is parked for the merchant to see.
        if matches!(payment.rail, crate::models::Rail::Onchain) {
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
                    "on-chain payment seen; EBP does not take on-chain payments, so it is parked in unmatched_payments"
                );
            }
            return Ok(false);
        }

        let address = payment.address.clone();
        let seen_at = payment.detected_at;
        let invoice = self
            .db
            .call(move |conn| invoice::find_creditable_by_address(conn, &address, seen_at))
            .await?;
        // R6: completed after the invoice's deadline (plus skew grace)?
        // Then it is a late payment even though `expire_due` has not
        // flipped the invoice yet; deterministic on the wallet's time.
        let mut late_reason: Option<&'static str> = None;
        // A payment that covers the whole invoice on an instant rail
        // (Ark, Lightning) still completes it for a few hours after the
        // deadline: the ASP honours Lightning invoices for 3 hours.
        let invoice = invoice.filter(|i| {
            if payment.detected_at > i.expires_at + invoice::LATE_PAYMENT_GRACE_SECS {
                let ok = invoice::late_full_payment_ok(
                    i,
                    payment.rail.as_str(),
                    credited_amount(&payment, i),
                    payment.detected_at,
                );
                if ok {
                    tracing::info!(
                        invoice_id = %i.id,
                        "full payment arrived after the deadline; completing the order"
                    );
                } else {
                    late_reason = Some("paid_after_expiry");
                }
                ok
            } else {
                true
            }
        });
        // An underpaid invoice accepts a top-up only inside the short
        // window after its first partial payment. The same holds once such an
        // invoice has expired: a late "full" payment never bypasses the window.
        let invoice = match invoice {
            Some(i)
                if i.status == crate::models::InvoiceStatus::Underpaid
                    || (i.status == crate::models::InvoiceStatus::Expired && i.paid_sats > 0) =>
            {
                let (id, seen) = (i.id.clone(), payment.detected_at);
                let closed = self
                    .db
                    .call(move |conn| invoice::topup_window_closed(conn, &id, seen))
                    .await?;
                if closed {
                    late_reason = Some("topup_window_closed");
                    None
                } else {
                    Some(i)
                }
            }
            other => other,
        };

        let Some(invoice) = invoice else {
            // BETA-HARDENING: no PENDING invoice owns this address. If it
            // belongs to an expired/closed invoice, the funds are in the
            // till and MUST be recorded (late payment) rather than
            // dropped. Money at an address of no payment request is
            // recorded too ("Money from elsewhere"), see `record_unmatched`.
            self.park(&payment, late_reason).await?;
            return Ok(false);
        };

        let store_id = invoice.store_id.clone();
        let observed = invoice::ObservedPayment {
            invoice_id: invoice.id.clone(),
            rail: payment.rail.as_str().to_string(),
            amount_sats: credited_amount(&payment, &invoice),
            received_sats: Some(payment.amount_sats),
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
                // This payment can never be credited to its order. Do not
                // lose sight of it: park it under Alerts so the owner sees
                // it and can refund or fulfil by hand.
                tracing::warn!(error = %err, "payment rejected as permanently unrecordable; parking it under Alerts");
                let (addr, rail) = (payment.address.clone(), payment.rail.as_str().to_string());
                let (vtxo, txid) = (payment.vtxo_id.clone(), payment.txid.clone());
                let (amount, seen) = (payment.amount_sats, payment.detected_at);
                self.db
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
                                reason_override: Some("could_not_be_recorded"),
                            },
                        )
                    })
                    .await?;
                return Ok(false);
            }
            Err(err) => {
                // Not a verdict on the payment: something on our side failed (a
                // busy database, a lost race). The caller notes the trouble; the
                // cursor stays put and the payment is tried again next tick.
                return Err(err);
            }
        };

        let Some((updated, status)) = outcome else {
            // Not counted. Either the database already holds this payment (a
            // repeat), or the invoice would not take it: for example a part
            // payment that reached an invoice which expired in the meantime.
            // The money is in the wallet either way, so it is written down
            // under Alerts unless it is already held somewhere. (Before, this
            // returned quietly and such a payment was never recorded.)
            let (vtxo, txid) = (payment.vtxo_id.clone(), payment.txid.clone());
            let held = self
                .db
                .call(move |conn| invoice::already_recorded(conn, vtxo.as_deref(), txid.as_deref()))
                .await?;
            if !held {
                self.park(&payment, late_reason.or(Some("paid_after_expiry")))
                    .await?;
            }
            return Ok(false);
        };

        tracing::info!(
            invoice_id = %updated.id,
            status = %status,
            amount_sats = updated.paid_sats,
            "payment applied"
        );

        self.emit_invoice_event(&store_id, &updated, status, true)
            .await;
        Ok(true)
    }

    /// Write a payment that is not counted to an order down under Alerts
    /// (`unmatched_payments`), so money in the wallet is never unexplained.
    async fn park(
        &self,
        payment: &WalletPayment,
        reason: Option<&'static str>,
    ) -> Result<(), AppError> {
        let (addr, rail) = (payment.address.clone(), payment.rail.as_str().to_string());
        let (vtxo, txid) = (payment.vtxo_id.clone(), payment.txid.clone());
        let (amount, seen) = (payment.amount_sats, payment.detected_at);
        let is_new = self
            .db
            .call(move |conn| {
                let is_new = invoice::record_unmatched(
                    conn,
                    &invoice::UnmatchedInput {
                        address: &addr,
                        rail: &rail,
                        amount_sats: amount,
                        vtxo_id: vtxo.as_deref(),
                        txid: txid.as_deref(),
                        detected_at: seen,
                        reason_override: reason,
                    },
                )?;
                // Money for an invoice from the Invoices tab that arrived after it
                // stopped taking payments: one phone notice, the first time only
                // (no amount, number or name, like every alert).
                if is_new && late_payment_is_for_manual_invoice(conn, &addr, amount, seen)? {
                    crate::alerts::queue_notice(conn, "invoice_paid_late")?;
                }
                Ok(is_new)
            })
            .await?;
        if is_new {
            tracing::warn!(
                amount_sats = payment.amount_sats,
                "late/unmatched payment recorded in unmatched_payments; merchant must reconcile"
            );
        }
        Ok(())
    }

    async fn emit_invoice_event(
        &self,
        store_id: &str,
        invoice: &crate::models::Invoice,
        status: InvoiceStatus,
        // True only right after a payment was saved. The gap-filling pass over
        // recently finished invoices calls this every check; shop events are
        // de-duplicated by the outbox, but the phone notice is not, so it is
        // only queued here (it repeated every minute in 0.3.3-rc2/rc3).
        new_payment: bool,
    ) {
        // An invoice the owner made on the Invoices tab is not a shop order:
        // no shop webhook. A payment that makes it paid queues one phone
        // notice instead (no amount, number or name, like every alert).
        let (inv_id, paid) = (
            invoice.id.clone(),
            new_payment && matches!(status, InvoiceStatus::Paid | InvoiceStatus::Overpaid),
        );
        match self
            .db
            .call(move |conn| {
                let manual = crate::manual_invoice::is_manual(conn, &inv_id)?;
                if manual && paid {
                    crate::alerts::queue_notice(conn, "invoice_paid")?;
                }
                Ok(manual)
            })
            .await
        {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(error = %err, "could not check the invoice's source; no webhook queued");
                return;
            }
        }
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
/// How much a payment pays toward its invoice.
///
/// A Lightning invoice has a fixed amount and can only be paid in full. The
/// customer was asked for the order amount plus the Ark server's receive fee,
/// and what the wallet reports can be that sum, the order amount, or a
/// rounding sat off. So a receive for this invoice's own Lightning invoice
/// that lands within the quoted fee (plus a small margin) of the order
/// amount pays the order exactly. Anything further off is counted as what
/// actually arrived, like every other payment, so a wildly wrong amount can
/// never mark an order paid.
fn credited_amount(
    payment: &crate::wallet::WalletPayment,
    invoice: &crate::models::Invoice,
) -> i64 {
    let lightning = payment
        .address
        .get(..2)
        .is_some_and(|p| p.eq_ignore_ascii_case("ln"));
    if !lightning {
        return payment.amount_sats;
    }
    let quoted_fee = serde_json::from_str::<serde_json::Value>(&invoice.metadata_json)
        .ok()
        .and_then(|m| m.get("lightning_fee_estimate_sat").and_then(|v| v.as_i64()))
        .unwrap_or(0)
        .max(0);
    let margin = quoted_fee + (invoice.amount_sats / 100).max(10);
    if (payment.amount_sats - invoice.amount_sats).abs() <= margin {
        if payment.amount_sats != invoice.amount_sats {
            tracing::info!(
                invoice_id = %invoice.id,
                received_sats = payment.amount_sats,
                order_sats = invoice.amount_sats,
                "lightning payment within the fee margin counted as paying the order in full"
            );
        }
        invoice.amount_sats
    } else {
        tracing::warn!(
            invoice_id = %invoice.id,
            received_sats = payment.amount_sats,
            order_sats = invoice.amount_sats,
            "lightning payment far from the order amount; counting what arrived"
        );
        payment.amount_sats
    }
}

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
    use crate::wallet::{AspStatus, ReceiveAddresses, WalletError, WalletHealth};

    struct FakeWallet {
        payments: Vec<WalletPayment>,
        vtxos: Vec<WalletVtxo>,
    }

    impl WalletAdapter for FakeWallet {
        fn receive_addresses(&self, invoice_id: &str) -> Result<ReceiveAddresses, WalletError> {
            Ok(ReceiveAddresses {
                ark_address: format!("ark1q{invoice_id}"),
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

    #[tokio::test]
    async fn a_payment_to_a_replaced_lightning_invoice_still_finds_its_order() {
        // A fresh Lightning invoice replaced the first one, but the customer
        // paid the first: the Ark server can accept it a while after its
        // written end. It must count toward the same order, not go missing.
        let (db, store_id) = setup(vec![]).await;
        db.call(|c| {
            invoice::set_lightning_invoice(c, "inv_1", "lntbs10030n1pold", None)?;
            invoice::set_lightning_invoice(c, "inv_1", "lntbs10030n1pnew", None)
        })
        .await
        .unwrap();
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![WalletPayment {
                    address: "lntbs10030n1pold".to_string(),
                    rail: Rail::Ark,
                    amount_sats: 1000,
                    vtxo_id: Some("ln_old:0".to_string()),
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
        assert_eq!(watcher.tick().await.unwrap(), 1, "credited to its order");
        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);
    }

    #[tokio::test]
    async fn a_lightning_payment_of_our_own_invoice_pays_the_order_in_full() {
        // The Ark server keeps its receive fee: 1003 asked (1000 + fee),
        // 998 arrived after rounding. The order is still paid, not underpaid.
        let (db, store_id) = setup(vec![]).await;
        db.call(|c| invoice::set_lightning_invoice(c, "inv_1", "lntbs10030n1pfake", None))
            .await
            .unwrap();
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![WalletPayment {
                    address: "lntbs10030n1pfake".to_string(),
                    rail: Rail::Ark,
                    amount_sats: 998,
                    vtxo_id: Some("ln_1:0".to_string()),
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
        assert_eq!(watcher.tick().await.unwrap(), 1);
        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);
        assert_eq!(invoice.paid_sats, 1000);
    }

    #[tokio::test]
    async fn an_address_counts_as_in_use_only_while_its_request_can_still_be_paid() {
        let (db, store_id) = setup(vec![]).await;
        let in_use = |db: Db, a: &'static str| async move {
            db.call(move |c| invoice::ark_address_in_use(c, a))
                .await
                .unwrap()
        };
        assert!(in_use(db.clone(), "ark1qinv_1").await);
        assert!(!in_use(db.clone(), "ark1qsomething_else").await);
        db.call(move |c| invoice::cancel(c, &store_id, "inv_1"))
            .await
            .unwrap();
        assert!(
            !in_use(db.clone(), "ark1qinv_1").await,
            "cancelled: free again"
        );
    }

    fn test_invoice() -> crate::models::Invoice {
        crate::models::Invoice {
            id: "inv_t".into(),
            store_id: "store_t".into(),
            order_id: None,
            status: InvoiceStatus::Pending,
            amount_sats: 0,
            fiat_currency: None,
            fiat_amount: None,
            rate: None,
            rate_source: None,
            rate_locked_at: None,
            ark_address: String::new(),
            expires_at: 0,
            paid_at: None,
            paid_sats: 0,
            confs_required: 1,
            metadata_json: "{}".into(),
            created_at: 0,
        }
    }

    #[test]
    fn a_lightning_amount_far_from_the_order_is_counted_as_what_arrived() {
        let ln = |sats: i64| WalletPayment {
            address: "lntbs1fake".to_string(),
            rail: Rail::Ark,
            amount_sats: sats,
            vtxo_id: None,
            txid: None,
            detected_at: 0,
            confirmed_at: None,
        };
        let invoice = |meta: &str| crate::models::Invoice {
            amount_sats: 100_000,
            metadata_json: meta.to_string(),
            ..test_invoice()
        };
        let plain = invoice("{}");
        // Within 1% (no fee quoted): the order is paid exactly.
        assert_eq!(credited_amount(&ln(99_500), &plain), 100_000);
        assert_eq!(credited_amount(&ln(100_400), &plain), 100_000);
        // Far off: never rounded up to "paid".
        assert_eq!(credited_amount(&ln(1), &plain), 1);
        assert_eq!(credited_amount(&ln(50_000), &plain), 50_000);
        assert_eq!(credited_amount(&ln(150_000), &plain), 150_000);
        // A quoted fee widens the margin by that fee.
        let with_fee = invoice(r#"{"lightning_fee_estimate_sat": 2000}"#);
        assert_eq!(credited_amount(&ln(102_000), &with_fee), 100_000);
        assert_eq!(credited_amount(&ln(97_500), &with_fee), 100_000);
        assert_eq!(credited_amount(&ln(96_000), &with_fee), 96_000);
        // Not Lightning: always what arrived.
        let mut ark = ln(99_999);
        ark.address = "tark1qqq".to_string();
        assert_eq!(credited_amount(&ark, &plain), 99_999);
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
    async fn money_that_arrives_on_chain_is_parked_never_credited() {
        // EBP takes no on-chain payments. An old invoice may still hold the
        // on-chain address it was given earlier; money sent there must be shown
        // to the merchant, not credited.
        let (db, store_id) = setup(vec![]).await;
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET onchain_address = 'bc1qinv_1' WHERE id = 'inv_1'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![onchain_payment()],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );

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
    async fn on_chain_money_to_an_address_no_invoice_owns_is_written_down_not_credited() {
        // For example money sent to an address of this server outside any
        // invoice: nothing is credited, and it is written down once as money
        // from elsewhere so the owner can see where it came from.
        let (db, store_id) = setup(vec![]).await;
        let mut stray = onchain_payment();
        stray.address = "bc1qsomeoneelse".to_string();
        let watcher = PollingWatcher::new(
            Arc::new(FakeWallet {
                payments: vec![stray],
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        );
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Pending);
        let parked: (i64, String) = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*), MAX(reason) FROM unmatched_payments WHERE invoice_id IS NULL",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(parked, (1, "unknown_address".to_string()));
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

    async fn queued_notices(db: &Db) -> String {
        db.call(|c| Ok(crate::settings::get(c, "alert_notices")?.unwrap_or_default()))
            .await
            .unwrap()
    }

    async fn trouble_showing(db: &Db) -> bool {
        db.call(|c| Ok(save_trouble_showing(c, now_unix())))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_payment_that_cannot_be_saved_shows_trouble_alerts_once_and_clears_when_saved() {
        let (db, store_id) = setup(vec![]).await;
        let watcher = late_watcher(&db, vec![ark_payment("vtxo_a", 1000)]);
        // Make the invoice update change nothing, as a lost race would.
        db.call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER no_update BEFORE UPDATE ON invoices BEGIN SELECT RAISE(IGNORE); END;",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // First failure: trouble shows on Home, but no phone alert yet.
        assert!(watcher.tick().await.is_err());
        assert!(trouble_showing(&db).await);
        assert_eq!(queued_notices(&db).await, "");

        // Still failing after five minutes: exactly one phone alert.
        db.call(|c| crate::settings::set(c, SAVE_TROUBLE_KEY, &(now_unix() - 400).to_string()))
            .await
            .unwrap();
        assert!(watcher.tick().await.is_err());
        assert!(watcher.tick().await.is_err());
        assert_eq!(queued_notices(&db).await, "payment_not_saved");
        db.call(|c| crate::settings::set(c, "alert_notices", ""))
            .await
            .unwrap();
        assert!(watcher.tick().await.is_err());
        assert_eq!(queued_notices(&db).await, "", "no repeat alert");

        // The cause goes away: the payment is saved and the trouble clears.
        db.call(|conn| {
            conn.execute_batch("DROP TRIGGER no_update;")?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert!(!trouble_showing(&db).await);
        let invoice = db
            .call(move |conn| invoice::get(conn, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(invoice.status, InvoiceStatus::Paid);
    }

    fn extra_invoice(store_id: &str, id: &str, expires_in: i64) -> NewInvoice {
        NewInvoice {
            id: id.to_string(),
            store_id: store_id.to_string(),
            order_id: Some(format!("wc-{id}")),
            amount_sats: 1000,
            fiat_currency: None,
            fiat_amount: None,
            rate: None,
            rate_source: None,
            rate_locked_at: None,
            ark_address: format!("ark1q{id}"),
            expires_at: now_unix() + expires_in,
            confs_required: 1,
            metadata_json: "{}".to_string(),
        }
    }

    async fn status_of(db: &Db, store_id: &str, id: &str) -> InvoiceStatus {
        let (store_id, id) = (store_id.to_string(), id.to_string());
        db.call(move |conn| invoice::get(conn, &store_id, &id))
            .await
            .unwrap()
            .unwrap()
            .status
    }

    #[tokio::test]
    async fn one_payment_that_cannot_be_saved_does_not_hold_up_the_others() {
        let (db, store_id) = setup(vec![]).await;
        let sid = store_id.clone();
        db.call(move |conn| {
            invoice::insert(conn, &extra_invoice(&sid, "inv_2", 1200))?;
            invoice::insert(conn, &extra_invoice(&sid, "inv_3", -10))
        })
        .await
        .unwrap();
        let mut second = ark_payment("vtxo_b", 1000);
        second.address = "ark1qinv_2".to_string();
        let watcher = late_watcher(&db, vec![ark_payment("vtxo_a", 1000), second]);
        // Only inv_1's update fails (as a lost race would).
        db.call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER no_update_1 BEFORE UPDATE ON invoices WHEN NEW.id = 'inv_1' BEGIN SELECT RAISE(IGNORE); END;",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        assert!(
            watcher.tick().await.is_err(),
            "the unsaved payment is still reported"
        );
        assert_eq!(
            status_of(&db, &store_id, "inv_2").await,
            InvoiceStatus::Paid
        );
        assert_eq!(
            status_of(&db, &store_id, "inv_1").await,
            InvoiceStatus::Pending
        );
        // Expiry still ran in the same check.
        assert_eq!(
            status_of(&db, &store_id, "inv_3").await,
            InvoiceStatus::Expired
        );
        assert!(trouble_showing(&db).await);
        // The cursor did not move past the unsaved payment.
        let cursor = db
            .call(|c| crate::db::get_setting(c, HISTORY_CURSOR_KEY))
            .await
            .unwrap();
        assert!(
            cursor.is_none(),
            "cursor stays put while a payment is unsaved"
        );

        // The cause goes away: the unsaved one is saved, the saved one is not doubled.
        db.call(|conn| {
            conn.execute_batch("DROP TRIGGER no_update_1;")?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(
            status_of(&db, &store_id, "inv_1").await,
            InvoiceStatus::Paid
        );
        let paid: i64 = db
            .call(|c| {
                Ok(c.query_row(
                    "SELECT paid_sats FROM invoices WHERE id = 'inv_2'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(paid, 1000, "not credited twice");
        assert!(!trouble_showing(&db).await);
    }

    #[tokio::test]
    async fn the_unsaved_payment_line_stays_for_as_long_as_the_trouble_does() {
        let (db, _) = setup(vec![]).await;
        // The trouble began two days ago and is still happening now.
        db.call(|c| {
            crate::settings::set(c, SAVE_TROUBLE_KEY, &(now_unix() - 172_800).to_string())?;
            note_save_trouble(c, now_unix())
        })
        .await
        .unwrap();
        assert!(trouble_showing(&db).await);
        // If it stopped being refreshed (the watcher itself is gone) it expires.
        db.call(|c| {
            crate::settings::set(
                c,
                SAVE_TROUBLE_SEEN_KEY,
                &(now_unix() - 172_800).to_string(),
            )
        })
        .await
        .unwrap();
        assert!(!trouble_showing(&db).await);
    }

    #[tokio::test]
    async fn a_short_hiccup_clears_without_any_phone_alert() {
        let (db, _) = setup(vec![]).await;
        let watcher = late_watcher(&db, vec![ark_payment("vtxo_a", 1000)]);
        db.call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER no_update BEFORE UPDATE ON invoices BEGIN SELECT RAISE(IGNORE); END;",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(watcher.tick().await.is_err());
        db.call(|conn| {
            conn.execute_batch("DROP TRIGGER no_update;")?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert!(!trouble_showing(&db).await);
        assert_eq!(queued_notices(&db).await, "");
    }

    #[tokio::test]
    async fn a_payment_that_can_never_be_counted_is_parked_under_alerts_not_skipped() {
        let (db, _) = setup(vec![]).await;
        // A zero-sat entry on an open invoice's address is refused for good.
        let watcher = late_watcher(&db, vec![ark_payment("vtxo_zero", 0)]);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let reason: String = db
            .call(|conn| {
                Ok(conn.query_row("SELECT reason FROM unmatched_payments", [], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(reason, "could_not_be_recorded");
        assert!(!trouble_showing(&db).await, "a verdict is not trouble");
    }

    /// Expire `inv_1` as if its 20-minute countdown ended `ago` seconds back.
    async fn expire_inv_1(db: &Db, ago: i64) {
        db.call(move |c| {
            c.execute(
                "UPDATE invoices SET status = 'expired', expires_at = ?1 WHERE id = 'inv_1'",
                [now_unix() - ago],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    fn late_watcher(db: &Db, payments: Vec<WalletPayment>) -> PollingWatcher {
        PollingWatcher::new(
            Arc::new(FakeWallet {
                payments,
                vtxos: vec![],
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        )
    }

    fn ark_payment(vtxo: &str, sats: i64) -> WalletPayment {
        WalletPayment {
            address: "ark1qinv_1".to_string(),
            rail: Rail::Ark,
            amount_sats: sats,
            vtxo_id: Some(vtxo.to_string()),
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        }
    }

    async fn count(db: &Db, sql: &'static str) -> i64 {
        db.call(move |c| Ok(c.query_row(sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_event_missed_earlier_is_queued_once_by_the_next_check() {
        let (db, store_id) = setup(vec![]).await;
        db.call(move |c| {
            store::set_webhook(c, &store_id, Some("https://shop.example.com/hook"), true)
                .map(|_| ())
        })
        .await
        .unwrap();
        // Expired by a page view (no event queued), as `expire_if_due` does.
        expire_inv_1(&db, 600).await;
        let watcher = late_watcher(&db, vec![]);
        watcher.tick().await.unwrap();
        watcher.tick().await.unwrap();
        let sql = "SELECT COUNT(*) FROM webhook_deliveries WHERE event = 'invoice.expired'";
        assert_eq!(count(&db, sql).await, 1, "queued once, never twice");
    }

    async fn status_and_paid(db: &Db) -> (String, i64) {
        db.call(|c| {
            Ok(c.query_row(
                "SELECT status, paid_sats FROM invoices WHERE id = 'inv_1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_part_payment_reaching_an_invoice_that_just_expired_is_written_down() {
        // Found by review (26 November 2026): a part payment seen inside the
        // 2-minute grace, or even before the deadline, on an invoice that had
        // already been flipped to expired, was refused by record_payment and
        // then dropped without a trace, every tick, forever.
        let (db, _) = setup(vec![]).await;
        expire_inv_1(&db, 30).await;
        let mut before_deadline = ark_payment("v_part_a", 300);
        before_deadline.detected_at = now_unix() - 60;
        let in_grace = ark_payment("v_part_b", 400);
        let watcher = late_watcher(&db, vec![before_deadline, in_grace]);
        assert_eq!(
            watcher.tick().await.unwrap(),
            0,
            "a part payment never revives an expired invoice"
        );
        let parked = "SELECT COUNT(*) FROM unmatched_payments WHERE invoice_id = 'inv_1' AND reason = 'paid_after_expiry'";
        assert_eq!(
            count(&db, parked).await,
            2,
            "both are written down with their invoice"
        );
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM payments").await,
            0,
            "and not counted"
        );
        // Remembered: written once, and the next check skips them.
        watcher.tick().await.unwrap();
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            2
        );
        assert_eq!(status_and_paid(&db).await, ("expired".to_string(), 0));
    }

    /// `inv_1` (1,000 sats) got a first part payment of 400 sats 20 minutes ago.
    async fn part_paid_20_minutes_ago(db: &Db) {
        db.call(|c| {
            let first = invoice::ObservedPayment {
                invoice_id: "inv_1".into(),
                rail: "ark".into(),
                amount_sats: 400,
                received_sats: Some(400),
                vtxo_id: Some("v_first".into()),
                txid: None,
                detected_at: now_unix() - 1_200,
                confirmed_at: None,
            };
            invoice::record_payment(c, &first)?.expect("counted");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_top_up_written_down_as_too_late_is_never_also_counted_after_expiry() {
        // Found by review (26 November 2026): after the invoice expired and the
        // hourly full pass (or a restart) looked at the parked top-up again, it
        // counted as a "full late payment" too: the same money in two places.
        let (db, _) = setup(vec![]).await;
        part_paid_20_minutes_ago(&db).await;
        let mut top_up = ark_payment("v_rest", 600);
        top_up.detected_at = now_unix() - 120; // 18 minutes after the first: window closed
        let watcher = late_watcher(&db, vec![top_up]);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let parked = "SELECT COUNT(*) FROM unmatched_payments WHERE reason = 'topup_window_closed'";
        assert_eq!(count(&db, parked).await, 1);
        // The invoice expires; then everything is looked at again.
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET status = 'expired', expires_at = ?1 WHERE id = 'inv_1'",
                [now_unix() - 30],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        watcher.handled.lock().unwrap().1 = 1;
        assert_eq!(
            watcher.tick().await.unwrap(),
            0,
            "not counted on the second look"
        );
        assert_eq!(status_and_paid(&db).await, ("expired".to_string(), 400));
        assert_eq!(count(&db, "SELECT COUNT(*) FROM payments").await, 1);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            1
        );
    }

    #[tokio::test]
    async fn a_late_top_up_first_seen_after_expiry_still_respects_the_window() {
        let (db, _) = setup(vec![]).await;
        part_paid_20_minutes_ago(&db).await;
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET status = 'expired', expires_at = ?1 WHERE id = 'inv_1'",
                [now_unix() - 30],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let mut top_up = ark_payment("v_rest", 600);
        top_up.detected_at = now_unix() - 120;
        let watcher = late_watcher(&db, vec![top_up]);
        assert_eq!(
            watcher.tick().await.unwrap(),
            0,
            "the top-up window applies after expiry too"
        );
        let parked = "SELECT COUNT(*) FROM unmatched_payments WHERE reason = 'topup_window_closed'";
        assert_eq!(count(&db, parked).await, 1);
        assert_eq!(status_and_paid(&db).await, ("expired".to_string(), 400));
    }

    #[tokio::test]
    async fn a_full_payment_just_after_the_deadline_still_completes_the_order() {
        // The fixes above must not stop the rule that a full payment inside
        // the late window completes an expired invoice.
        let (db, _) = setup(vec![]).await;
        expire_inv_1(&db, 30).await;
        let watcher = late_watcher(&db, vec![ark_payment("v_full", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(status_and_paid(&db).await, ("paid".to_string(), 1000));
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            0
        );
    }

    #[tokio::test]
    async fn a_late_payment_for_an_invoice_from_the_invoices_tab_notifies_once() {
        let (db, _store_id) = setup(vec![]).await;
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET source = 'manual' WHERE id = 'inv_1'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        // Long past the late window: parked under Alerts, not counted.
        expire_inv_1(&db, crate::invoice::LATE_FULL_PAYMENT_SECS + 3_600).await;
        let watcher = late_watcher(&db, vec![ark_payment("v_late", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 0, "not counted");
        let notices = || {
            let db = db.clone();
            async move {
                db.call(|c| crate::settings::get(c, "alert_notices"))
                    .await
                    .unwrap()
                    .unwrap_or_default()
            }
        };
        assert_eq!(notices().await, "invoice_paid_late");
        let sql = "SELECT COUNT(*) FROM unmatched_payments WHERE invoice_id = 'inv_1'";
        assert_eq!(count(&db, sql).await, 1, "written down with the invoice");
        db.call(|c| crate::settings::set(c, "alert_notices", ""))
            .await
            .unwrap();
        for _ in 0..3 {
            watcher.tick().await.unwrap();
        }
        assert_eq!(notices().await, "", "never queued again");
    }

    #[tokio::test]
    async fn a_paid_invoice_from_the_invoices_tab_notifies_the_phone_once() {
        let (db, _store_id) = setup(vec![]).await;
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET source = 'manual' WHERE id = 'inv_1'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let notices = |db: Db| async move {
            db.call(|c| crate::settings::get(c, "alert_notices"))
                .await
                .unwrap()
                .unwrap_or_default()
        };
        let watcher = late_watcher(&db, vec![ark_payment("v_inv", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 1, "credited");
        assert_eq!(
            notices(db.clone()).await,
            "invoice_paid",
            "one notice queued"
        );
        // The alert check takes the notice; later checks must not queue it again.
        db.call(|c| crate::settings::set(c, "alert_notices", ""))
            .await
            .unwrap();
        for _ in 0..3 {
            watcher.tick().await.unwrap();
        }
        assert_eq!(notices(db.clone()).await, "", "never queued again");
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM webhook_deliveries").await,
            0,
            "no shop event for an invoice made by hand"
        );
    }

    #[tokio::test]
    async fn full_payment_after_expiry_completes_the_order() {
        let (db, store_id) = setup(vec![]).await;
        expire_inv_1(&db, 600).await;
        let watcher = late_watcher(&db, vec![ark_payment("v_late", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 1, "credited");
        let inv = db
            .call(move |c| invoice::get(c, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::Paid);
        assert_eq!(inv.paid_sats, 1000);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            0
        );
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM audit_log WHERE action = 'invoice.paid_late'"
            )
            .await,
            1
        );
        // Polling the same history again changes nothing.
        assert_eq!(watcher.tick().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn part_payment_after_expiry_is_still_parked() {
        let (db, store_id) = setup(vec![]).await;
        expire_inv_1(&db, 600).await;
        let watcher = late_watcher(&db, vec![ark_payment("v_part", 400)]);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let inv = db
            .call(move |c| invoice::get(c, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::Expired);
        assert_eq!(inv.paid_sats, 0);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            1
        );
    }

    #[tokio::test]
    async fn full_payment_long_after_expiry_is_parked() {
        let (db, store_id) = setup(vec![]).await;
        expire_inv_1(&db, 4 * 3600).await; // past the 3-hour late window
        let watcher = late_watcher(&db, vec![ark_payment("v_vlate", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let inv = db
            .call(move |c| invoice::get(c, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::Expired);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            1
        );
    }

    #[tokio::test]
    async fn second_full_payment_after_a_late_completion_is_parked_as_extra() {
        let (db, store_id) = setup(vec![]).await;
        expire_inv_1(&db, 600).await;
        let watcher = late_watcher(
            &db,
            vec![ark_payment("v_one", 1000), ark_payment("v_two", 1000)],
        );
        assert_eq!(watcher.tick().await.unwrap(), 1, "only the first counts");
        let inv = db
            .call(move |c| invoice::get(c, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::Paid);
        assert_eq!(inv.paid_sats, 1000, "never credited twice");
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            1
        );
    }

    #[tokio::test]
    async fn cancelled_invoice_is_never_revived() {
        let (db, store_id) = setup(vec![]).await;
        db.call(|c| {
            c.execute(
                "UPDATE invoices SET status = 'cancelled' WHERE id = 'inv_1'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let watcher = late_watcher(&db, vec![ark_payment("v_c", 1000)]);
        assert_eq!(watcher.tick().await.unwrap(), 0);
        let inv = db
            .call(move |c| invoice::get(c, &store_id, "inv_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv.status, InvoiceStatus::Cancelled);
        assert_eq!(
            count(&db, "SELECT COUNT(*) FROM unmatched_payments").await,
            1
        );
    }

    #[tokio::test]
    async fn tick_records_money_from_elsewhere_without_crediting() {
        let (db, _) = setup(vec![]).await;
        let db2 = db.clone();
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

        assert_eq!(watcher.tick().await.unwrap(), 0, "credits nothing");
        watcher.tick().await.unwrap();
        let sql = "SELECT COUNT(*) FROM unmatched_payments
                   WHERE invoice_id IS NULL AND reason = 'unknown_address'";
        assert_eq!(count(&db2, sql).await, 1, "written down once, under Alerts");
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

    fn quiet_watcher(db: Db, payments: Vec<WalletPayment>) -> PollingWatcher {
        PollingWatcher::new(
            Arc::new(FakeWallet {
                payments,
                vtxos: vec![],
            }),
            db,
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
            604_800,
        )
    }

    fn remembered(w: &PollingWatcher) -> usize {
        w.handled.lock().unwrap().0.len()
    }

    #[tokio::test]
    async fn an_entry_the_database_holds_is_remembered_and_then_skipped() {
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
        let watcher = quiet_watcher(db.clone(), vec![payment]);
        assert_eq!(remembered(&watcher), 0);
        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(remembered(&watcher), 1, "recorded, so remembered");
        assert_eq!(watcher.tick().await.unwrap(), 0);
        assert_eq!(remembered(&watcher), 1);
    }

    #[tokio::test]
    async fn an_entry_the_database_does_not_hold_is_never_remembered() {
        let (db, _) = setup(vec![]).await;
        // Money at an address of no payment request with no VTXO id or txid:
        // it cannot be written down safely (it would repeat), so it is not.
        let stranger = WalletPayment {
            address: "ark1qsomeone_else".to_string(),
            rail: Rail::Ark,
            amount_sats: 5000,
            vtxo_id: None,
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        };
        let watcher = quiet_watcher(db, vec![stranger]);
        for _ in 0..3 {
            assert_eq!(watcher.tick().await.unwrap(), 0);
        }
        assert_eq!(
            remembered(&watcher),
            0,
            "entries not in the database are looked at every time"
        );
    }

    #[tokio::test]
    async fn money_from_elsewhere_is_written_down_and_then_remembered() {
        let (db, _) = setup(vec![]).await;
        let stranger = WalletPayment {
            address: "ark1qsomeone_else".to_string(),
            rail: Rail::Ark,
            amount_sats: 5000,
            vtxo_id: Some("vtxo_x".to_string()),
            txid: None,
            detected_at: now_unix(),
            confirmed_at: None,
        };
        let watcher = quiet_watcher(db.clone(), vec![stranger]);
        for _ in 0..3 {
            assert_eq!(watcher.tick().await.unwrap(), 0, "never credited");
        }
        assert_eq!(
            remembered(&watcher),
            1,
            "held in the database, so skipped next time"
        );
        let sql = "SELECT COUNT(*) FROM unmatched_payments WHERE reason = 'unknown_address'";
        assert_eq!(count(&db, sql).await, 1);
    }

    #[tokio::test]
    async fn the_remembered_set_is_emptied_for_a_full_pass_after_an_hour() {
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
        let watcher = quiet_watcher(db, vec![payment]);
        watcher.tick().await.unwrap();
        assert_eq!(remembered(&watcher), 1);
        // Pretend the last full pass was long ago: the next tick forgets,
        // reads everything again (a harmless no-op) and remembers it afresh.
        watcher.handled.lock().unwrap().1 = 1;
        assert_eq!(watcher.tick().await.unwrap(), 0);
        assert_eq!(remembered(&watcher), 1);
        assert!(watcher.handled.lock().unwrap().1 > 1_000_000);
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
