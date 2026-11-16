-- Migration 0010 — cookie sessions for the admin console.
--
-- The raw session token (32 random bytes, base64url) is only ever sent to the
-- browser: the database stores its SHA-256 hexadecimal digest. `expires_at` is
-- an absolute deadline (7 days after creation) and every timestamp is in
-- milliseconds since the Unix epoch.
CREATE TABLE admin_sessions (
    token_hash      TEXT    PRIMARY KEY,
    method          TEXT    NOT NULL CHECK (method IN ('discord', 'secret')),
    discord_user_id TEXT,
    display_name    TEXT,
    avatar_url      TEXT,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    last_seen_at    INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_admin_sessions_expires_at ON admin_sessions (expires_at);
