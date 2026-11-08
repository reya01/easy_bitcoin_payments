-- The watcher looks a payment up by Lightning invoice text on every pass over
-- the wallet history (every few seconds, for every entry). Without an index
-- each lookup scans the whole table, so the work grows with the square of the
-- number of payments. Safe to add: it changes no data.
CREATE INDEX IF NOT EXISTS idx_ln_invoices_bolt11 ON ln_invoices(bolt11);
