-- Keep what the wallet actually received for each payment.
--
-- `amount_sats` is what was CREDITED to the order. For a Lightning payment of
-- the shop's own invoice that is the order amount whenever the wallet's figure
-- is within the fee margin of it, so the two can differ by a few sats. The
-- wallet's own figure was only written to the log; keeping it here lets the
-- payments export match the wallet exactly.
--
-- Older rows have NULL: they were credited as received.
ALTER TABLE payments ADD COLUMN received_sats INTEGER;
