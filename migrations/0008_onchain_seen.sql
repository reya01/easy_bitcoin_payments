-- On-chain outputs paying an invoice address that EBP has seen (mempool or
-- mined). Used to (a) remember when a payment was FIRST seen, so a payment
-- broadcast before the deadline is not "late" just because it confirms
-- after it, and (b) keep such an invoice from expiring while it waits for
-- its confirmation. Crediting still requires the configured confirmations.
CREATE TABLE onchain_seen (
    invoice_id    TEXT    NOT NULL REFERENCES invoices(id),
    txid          TEXT    NOT NULL,
    vout          INTEGER NOT NULL,
    amount_sats   INTEGER NOT NULL,
    first_seen_at INTEGER NOT NULL,
    confirmations INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (txid, vout)
);
CREATE INDEX idx_onchain_seen_invoice ON onchain_seen(invoice_id);
