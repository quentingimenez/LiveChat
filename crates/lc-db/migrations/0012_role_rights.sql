-- Migration 0012 — configurable rights per surface.
--
-- A missing `role_rights` row means "use the built-in default": the defaults
-- reproduce the historical hard-coded rules (overlay `GuildRole::can`, the
-- extension member/admin gates and the Discord command requirements). Fixed
-- cells (the `admin` column of `overlay` and `extension`, `receiveMedia` for
-- every role and `discord.info` for every role) are not stored: the server
-- returns and enforces their fixed value and refuses to write them.
--
-- `server_flags` keeps the `discordAdminIsAdmin` switch (a Discord server
-- administrator counts as the `admin` role). A missing row means the default
-- `true`. Values are the lowercase strings `true` / `false` so the flag stays
-- readable with a plain SQLite client.
CREATE TABLE role_rights (
    surface TEXT    NOT NULL CHECK (surface IN ('overlay', 'extension', 'discord')),
    action  TEXT    NOT NULL,
    role    TEXT    NOT NULL CHECK (role IN ('none', 'guest', 'member', 'admin')),
    allowed INTEGER NOT NULL CHECK (allowed IN (0, 1)),
    PRIMARY KEY (surface, action, role)
) STRICT;

CREATE TABLE server_flags (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
