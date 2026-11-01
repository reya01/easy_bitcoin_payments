-- Fix the on-chain idempotency guard.
--
-- 0001 made `txid` globally unique. A single on-chain transaction may
-- legitimately pay several invoices at once (one transaction, several
-- outputs, each to a different invoice address); the global guard silently
-- dropped every payment after the first, leaving a paid invoice stuck in
-- `pending`. Uniqueness is per (invoice_id, txid) instead: the same output
-- can never be credited twice to the same invoice, while a multi-output
-- transaction still credits every invoice it pays. The VTXO guard is
-- unchanged — one VTXO is one coin paying one address.
DROP INDEX IF EXISTS idx_payments_txid;
CREATE UNIQUE INDEX idx_payments_txid ON payments(invoice_id, txid) WHERE txid IS NOT NULL;