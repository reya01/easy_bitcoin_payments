-- Browser login for the merchant admin page. One admin account (the store
-- owner); the password hash is PBKDF2-HMAC-SHA256 (salt + iterations inside
-- the string). Sessions are stored HASHED, like API keys.
CREATE TABLE admin_account (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    password_hash TEXT    NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE admin_sessions (
    token_hash TEXT    PRIMARY KEY,
    csrf       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
);
