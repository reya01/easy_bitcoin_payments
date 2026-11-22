-- The Bitcoin price when a transfer or upkeep line was written, for the owner's
-- bookkeeping (the transfers file). Filled only when a recent price is known;
-- lines written before this migration stay blank rather than guessed.
ALTER TABLE settlements ADD COLUMN fiat_currency TEXT;
ALTER TABLE settlements ADD COLUMN fiat_amount   TEXT;
ALTER TABLE settlements ADD COLUMN rate          TEXT;
ALTER TABLE settlements ADD COLUMN rate_source   TEXT;
