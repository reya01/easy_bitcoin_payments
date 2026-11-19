-- Passkeys (WebAuthn) as a second step next to the authenticator app.
-- Nothing secret is stored: credential_id and public_key are public values.
-- public_key is the 65-byte uncompressed P-256 point (0x04 || x || y).
-- sign_count follows the authenticator's use counter (stays 0 for synced
-- passkeys such as a password manager's). backup_eligible / backed_up are the
-- authenticator's own flags, kept so the Settings page can say "synced" or
-- "only on this device".
CREATE TABLE admin_passkeys (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    credential_id   TEXT    NOT NULL UNIQUE,
    public_key      BLOB    NOT NULL,
    sign_count      INTEGER NOT NULL DEFAULT 0,
    name            TEXT    NOT NULL,
    backup_eligible INTEGER NOT NULL DEFAULT 0,
    backed_up       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    last_used_at    INTEGER
);
