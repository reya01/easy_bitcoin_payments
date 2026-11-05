-- Beta hardening: never silently drop money.
--
-- A wallet payment that arrives at a known EBP address but cannot be applied
-- to a pending invoice (invoice already expired / cancelled / paid, or the
-- amount was rejected) used to be logged at debug level and discarded. The
-- funds were still in the till with no record. This table is the audit trail
-- for the merchant: reconcile manually (refund or re-credit), then set
-- resolved_at.
CREATE TABLE unmatched_payments (
    id          TEXT PRIMARY KEY,
    address     TEXT NOT NULL,
    invoice_id  TEXT,               -- set when the address maps to a non-pending invoice
    rail        TEXT NOT NULL,
    amount_sats INTEGER NOT NULL,
    vtxo_id     TEXT,
    txid        TEXT,
    reason      TEXT NOT NULL,      -- e.g. invoice_expired | invoice_closed | unknown_address
    detected_at INTEGER NOT NULL,
    created_at  INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE UNIQUE INDEX idx_unmatched_vtxo ON unmatched_payments(vtxo_id) WHERE vtxo_id IS NOT NULL;
CREATE UNIQUE INDEX idx_unmatched_txid ON unmatched_payments(txid) WHERE txid IS NOT NULL;
