-- Phase 1: settlement engine, VTXO expiry queue, watcher cursor, LN invoices.
--
-- The expiry queue is driven by barkd's real per-VTXO expiries
-- (learn_lifetime.md): board/refresh VTXOs live ~28 days, Lightning-received
-- VTXOs only 1-3 days, and spend VTXOs inherit their parent's expiry.

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE vtxo_expiry (
    id               TEXT PRIMARY KEY,   -- barkd's VTXO id/outpoint
    amount_sats      INTEGER NOT NULL,
    expiry_at        INTEGER NOT NULL,   -- unix seconds; 0 = unknown (urgent)
    renew_by         INTEGER NOT NULL,   -- expiry_at - renewal lead time
    likely_lightning INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'live', -- live | gone
    updated_at       INTEGER NOT NULL
);

CREATE INDEX idx_vtxo_expiry_queue ON vtxo_expiry(status, renew_by);

-- Every sweep/renewal the settlement engine attempts. Statuses follow the
-- spec's settlements model; Level 0 sweeps are automatic, so nothing sits in
-- 'proposed' or 'awaiting_approval'.
CREATE TABLE settlements (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,          -- sweep_ark | sweep_onchain | renew
    amount_sats  INTEGER NOT NULL,
    destination  TEXT NOT NULL,
    status       TEXT NOT NULL,          -- broadcast | failed
    detail       TEXT,
    created_at   INTEGER NOT NULL,
    confirmed_at INTEGER
);

CREATE INDEX idx_settlements_created ON settlements(created_at);

-- Lightning invoices issued for checkout through the ASP gateway. The
-- identifier is barkd's handle for GET /lightning/receives/{identifier}.
CREATE TABLE ln_invoices (
    invoice_id  TEXT PRIMARY KEY REFERENCES invoices(id) ON DELETE CASCADE,
    bolt11      TEXT NOT NULL,
    identifier  TEXT,
    created_at  INTEGER NOT NULL
);