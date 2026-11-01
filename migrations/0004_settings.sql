-- Phase 1 close-out: server-local key/value settings — wizard progress
-- (setup_wallet_created, setup_backup_confirmed, setup_token_verified,
-- setup_complete) and, later, policies and notification config.
--
-- No secrets ever live here: the barkd token is env-only (EBP_BARKD_TOKEN)
-- and the wallet mnemonic is never stored at all.
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);