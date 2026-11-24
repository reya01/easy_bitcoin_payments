-- Owner-chosen labels for an invoice, for matching and analysis later
-- (docs/dev/INVOICES.md, "Identifiers for matching and analysis").
--
-- `invoice_number` is free text (at most 40 characters, checked in
-- src/invoice_ident.rs). It is unique across the server, ignoring upper and
-- lower case, so a number always points to exactly one invoice. NULL means
-- "no number" (every shop order made before or without one); NULLs never
-- clash with each other. The next default number is worked out from the
-- stored numbers, never kept separately, so a restored backup cannot hand out
-- a number twice.
--
-- `customer_name` is free text (at most 100 characters) and not unique: one
-- customer has many invoices.
--
-- Both stay in this database and its encrypted backups only: never in logs or
-- phone alerts.
ALTER TABLE invoices ADD COLUMN invoice_number TEXT;
ALTER TABLE invoices ADD COLUMN customer_name  TEXT;
CREATE UNIQUE INDEX idx_invoices_invoice_number ON invoices(invoice_number COLLATE NOCASE);
