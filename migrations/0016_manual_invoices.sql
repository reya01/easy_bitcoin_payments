-- Invoices the owner makes by hand on the Invoices tab (docs/dev/INVOICES.md),
-- as opposed to payment requests a shop makes at checkout.
--
-- `source` is 'shop' for every payment request a shop makes (and every row
-- made before this migration) and 'manual' for the Invoices tab. Manual
-- invoices send no shop webhook. `note` is the owner's short "what it is for"
-- text, shown in the payment request; kept only in this database and its
-- encrypted backups, never in logs or phone alerts.
ALTER TABLE invoices ADD COLUMN source TEXT NOT NULL DEFAULT 'shop';
ALTER TABLE invoices ADD COLUMN note   TEXT;
CREATE INDEX idx_invoices_source_created ON invoices(source, created_at);
