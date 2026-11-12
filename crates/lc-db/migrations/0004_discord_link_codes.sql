-- Codes de liaison Discord : émis par un admin d'une guilde indépendante
-- depuis l'overlay, saisis par un administrateur du serveur Discord dans la
-- commande slash `/lier` (`/link`). Usage unique, durée de vie courte.
CREATE TABLE discord_link_codes (
    code                 TEXT    PRIMARY KEY,
    guild_id             TEXT    NOT NULL,
    created_by_member_id TEXT,
    expires_at           INTEGER NOT NULL,
    used_at              INTEGER,
    created_at           INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_discord_link_codes_guild ON discord_link_codes (guild_id);
