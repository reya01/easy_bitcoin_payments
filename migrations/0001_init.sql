-- Phase 0 schema: stores, api_keys, invoices, payments.
--
-- Deliberately out of scope for Phase 0 (arrive in Phase 1/2):
--   settlements, webhook_deliveries, vtxo_expiry, audit_log, settings.
--
-- Custody note: `stores.webhook_secret` is stored in plaintext today because
-- the emitter must sign with it. At-rest encryption using
-- `EBP_KEY_ENCRYPTION_KEY` is a documented Phase 1 TODO; the config field and
-- this column exist so the change is additive. Secrets are never logged.

CREATE TABLE stores (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    webhook_url         TEXT,
    webhook_secret_hash TEXT,
    webhook_secret      TEXT,
    policy_json         TEXT NOT NULL DEFAULT '{}',
    created_at          INTEGER NOT NULL
);

CREATE TABLE api_keys (
    id          TEXT PRIMARY KEY,
    store_id    TEXT NOT NULL REFERENCES stores(id) ON DELETE CASCADE,
    key_hash    TEXT NOT NULL UNIQUE,
    label       TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    revoked_at  INTEGER
);

CREATE INDEX idx_api_keys_store ON api_keys(store_id);

CREATE TABLE invoices (
    id              TEXT PRIMARY KEY,
    store_id        TEXT NOT NULL REFERENCES stores(id) ON DELETE CASCADE,
    order_id        TEXT,
    status          TEXT NOT NULL,
    amount_sats     INTEGER NOT NULL,
    fiat_currency   TEXT,
    fiat_amount     TEXT,
    rate            TEXT,
    rate_source     TEXT,
    rate_locked_at  INTEGER,
    ark_address     TEXT NOT NULL,
    onchain_address TEXT NOT NULL,
    expires_at      INTEGER NOT NULL,
    paid_at         INTEGER,
    paid_sats       INTEGER NOT NULL DEFAULT 0,
    confs_required  INTEGER NOT NULL DEFAULT 1,
    metadata_json   TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_invoices_store_status ON invoices(store_id, status);
CREATE INDEX idx_invoices_ark_address ON invoices(ark_address);
CREATE INDEX idx_invoices_onchain_address ON invoices(onchain_address);
CREATE INDEX idx_invoices_expires_at ON invoices(expires_at);

CREATE TABLE payments (
    id           TEXT PRIMARY KEY,
    invoice_id   TEXT NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    rail         TEXT NOT NULL,
    amount_sats  INTEGER NOT NULL,
    vtxo_id      TEXT,
    txid         TEXT,
    detected_at  INTEGER NOT NULL,
    confirmed_at INTEGER
);

CREATE INDEX idx_payments_invoice ON payments(invoice_id);

-- Idempotency guards: a VTXO or txid can only ever be credited once.
CREATE UNIQUE INDEX idx_payments_vtxo ON payments(vtxo_id) WHERE vtxo_id IS NOT NULL;
CREATE UNIQUE INDEX idx_payments_txid ON payments(txid) WHERE txid IS NOT NULL;