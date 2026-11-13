-- A `/pair-code` code issued by a Discord administrator in normal mode grants
-- the LiveChat `admin` role when it is consumed. The decision is frozen at
-- pairing time: existing members are never re-synced from Discord permissions.
ALTER TABLE pairing_codes ADD COLUMN grants_admin INTEGER NOT NULL
    DEFAULT 0 CHECK (grants_admin IN (0, 1));
