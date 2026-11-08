-- Two-step sign-in with an authenticator app (TOTP, RFC 6238).
-- secret_enc is the shared secret, encrypted like other stored secrets.
-- enabled = 0 means "shown to the user, not yet confirmed with a code".
-- last_step blocks re-using a code; fail_count / locked_until slow down guessing.
CREATE TABLE admin_totp (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    secret_enc   TEXT    NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 0,
    last_step    INTEGER NOT NULL DEFAULT 0,
    fail_count   INTEGER NOT NULL DEFAULT 0,
    locked_until INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);

-- One-time recovery codes, stored hashed (like API keys).
CREATE TABLE admin_recovery_codes (
    code_hash TEXT PRIMARY KEY,
    used_at   INTEGER
);
