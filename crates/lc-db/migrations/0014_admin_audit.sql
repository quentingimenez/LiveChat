-- Migration 0014 — persistent audit log of admin mutations.
--
-- Only the stable action name, HTTP method, path (never the query string),
-- response status, actor (when a session authenticated the request), optional
-- guild id parsed from the path and the client IP are stored. Headers and
-- request bodies are deliberately absent: they may carry secrets.
--
-- `at` is an epoch-milliseconds timestamp (project convention).
CREATE TABLE admin_audit (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           INTEGER NOT NULL,
    actor_method TEXT,
    actor_name   TEXT,
    action       TEXT    NOT NULL,
    method       TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    status       INTEGER NOT NULL,
    guild_id     TEXT,
    client_ip    TEXT
) STRICT;

CREATE INDEX idx_admin_audit_at ON admin_audit (at);
