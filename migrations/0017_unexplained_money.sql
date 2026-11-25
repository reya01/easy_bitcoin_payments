-- Every Lightning invoice ever issued for a payment request, not only the
-- newest (`ln_invoices` keeps one per request and a fresh one replaces it).
-- A payment to a replaced Lightning invoice still finds its request.
CREATE TABLE ln_invoice_history (
    bolt11      TEXT PRIMARY KEY,
    invoice_id  TEXT NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL
);
CREATE INDEX idx_ln_invoice_history_invoice ON ln_invoice_history(invoice_id);
INSERT OR IGNORE INTO ln_invoice_history (bolt11, invoice_id, created_at)
    SELECT bolt11, invoice_id, created_at FROM ln_invoices;

-- Addresses of this server handed out for something other than a payment
-- request, so money that arrives there can be named. Today: the address
-- shown for the emergency withdrawal's fee money (`purpose` 'exit_fees').
CREATE TABLE own_addresses (
    address     TEXT PRIMARY KEY,
    purpose     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
