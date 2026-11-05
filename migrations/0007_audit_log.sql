-- Security-relevant actions per store (spec: audit_log). Details never hold
-- secrets: only what changed ("webhook url set", "api keys rotated"), not values.
CREATE TABLE audit_log (
    id         TEXT PRIMARY KEY,
    store_id   TEXT NOT NULL,
    action     TEXT NOT NULL,
    detail     TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX idx_audit_log_store ON audit_log(store_id, created_at);
