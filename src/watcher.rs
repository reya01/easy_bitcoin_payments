//! Payment watcher skeleton.
//!
//! Polling today; a subscription adapter can be added later behind the same
//! `tick` contract. The loop, address matching, state transitions, and webhook
//! emission are real and unit-tested — only the wallet source is stubbed.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use crate::db::Db;
use crate::error::AppError;
use crate::invoice;
use crate::models::InvoiceStatus;
use crate::store;
use crate::util::{now_unix, to_rfc3339};
use crate::wallet::{WalletAdapter, WalletPayment};
use crate::webhook::WebhookEmitter;

pub struct PollingWatcher {
    wallet: Arc<dyn WalletAdapter>,
    db: Db,
    webhooks: Arc<WebhookEmitter>,
    interval: Duration,
    cursor: Mutex<i64>,
}

impl PollingWatcher {
    pub fn new(
        wallet: Arc<dyn WalletAdapter>,
        db: Db,
        webhooks: Arc<WebhookEmitter>,
        interval: Duration,
    ) -> Self {
        Self {
            wallet,
            db,
            webhooks,
            interval,
            cursor: Mutex::new(0),
        }
    }

    /// Run forever, polling on the configured interval.
    pub async fn run(self) {
        let mut ticker = tokio::time::interval(self.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            match self.tick().await {
                Ok(0) => {}
                Ok(applied) => tracing::info!(applied, "watcher applied payments"),
                Err(err) => tracing::warn!(error = %err, "watcher tick failed"),
            }
        }
    }

    /// One poll cycle. Returns the number of payments applied to invoices.
    pub async fn tick(&self) -> Result<usize, AppError> {
        let since = *self
            .cursor
            .lock()
            .map_err(|_| AppError::Internal("watcher cursor poisoned".into()))?;

        let payments = self
            .wallet
            .poll_payments(since)
            .map_err(|e| AppError::Internal(format!("wallet poll failed: {e}")))?;

        let mut applied = 0usize;
        let mut newest = since;

        for payment in payments {
            newest = newest.max(payment.detected_at);

            let address = payment.address.clone();
            let invoice = self
                .db
                .call(move |conn| invoice::find_pending_by_address(conn, &address))
                .await?;

            let Some(invoice) = invoice else {
                tracing::debug!("payment at unknown address ignored");
                continue;
            };

            let invoice_id = invoice.id.clone();
            let store_id = invoice.store_id.clone();
            let rail = payment.rail.as_str().to_string();
            let amount = payment.amount_sats;
            let vtxo_id = payment.vtxo_id.clone();
            let txid = payment.txid.clone();
            let detected_at = payment.detected_at;
            let confirmed_at = payment.confirmed_at;

            let outcome = self
                .db
                .call(move |conn| {
                    invoice::record_payment(
                        conn,
                        &invoice_id,
                        &rail,
                        amount,
                        vtxo_id.as_deref(),
                        txid.as_deref(),
                        detected_at,
                        confirmed_at,
                    )
                })
                .await?;

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

        // Expire invoices whose rate lock has lapsed even when no checkout
        // page is open to notice, so stores receive `invoice.expired` and
        // `GET /api/v1/invoices/{id}` stops reporting `pending` for dead
        // invoices.
        let expired = self.db.call(|conn| invoice::expire_due(conn)).await?;
        for invoice in expired {
            tracing::info!(invoice_id = %invoice.id, "invoice expired");
            self.emit_invoice_event(&invoice.store_id, &invoice, InvoiceStatus::Expired)
                .await;
        }

        if let Ok(mut cursor) = self.cursor.lock() {
            *cursor = newest.max(now_unix() - 1);
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

        let (Some(url), Some(secret)) = (store.webhook_url, store.webhook_secret) else {
            return;
        };

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

        if let Err(err) = self
            .webhooks
            .emit(&url, &secret, event, &idempotency_key, &payload)
            .await
        {
            // Phase 1 adds the delivery log and retry schedule.
            tracing::warn!(error = %err, event, "webhook delivery failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoice::NewInvoice;
    use crate::models::Rail;
    use crate::wallet::{AspStatus, ReceiveAddresses, WalletError, WalletHealth};

    struct FakeWallet {
        payments: Vec<WalletPayment>,
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
    }

    async fn setup(payments: Vec<WalletPayment>) -> (Db, String) {
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
            }),
            db.clone(),
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
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
            }),
            db,
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
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
            }),
            db,
            Arc::new(WebhookEmitter::new(1)),
            Duration::from_secs(1),
        );

        assert_eq!(watcher.tick().await.unwrap(), 1);
        assert_eq!(watcher.tick().await.unwrap(), 0);
    }
}