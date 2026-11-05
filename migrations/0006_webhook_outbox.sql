-- Durable webhook outbox (beta hardening).
--
-- The watcher used to send webhooks inline / from an in-memory retry task, so
-- a restart or a merchant outage could lose `invoice.paid` forever. Events are
-- now written here in the same process that observes the payment and
-- delivered by `webhook_outbox::run` with exponential backoff; delivery state
-- survives restarts. `idempotency_key` is UNIQUE so re-observing the same
-- payment cannot enqueue the event twice.
CREATE TABLE webhook_deliveries (
    id               TEXT PRIMARY KEY,
    store_id         TEXT NOT NULL,
    invoice_id       TEXT NOT NULL,
    event            TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL UNIQUE,
    payload_json     TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'pending', -- pending | delivered | failed
    attempts         INTEGER NOT NULL DEFAULT 0,
    next_attempt_at  INTEGER NOT NULL,
    last_error       TEXT,
    created_at       INTEGER NOT NULL,
    delivered_at     INTEGER
);
CREATE INDEX idx_webhook_deliveries_due ON webhook_deliveries(status, next_attempt_at);
