-- Migration 0003 — guildes indépendantes de Discord, membres, rôles, invitations.
--
-- Jusqu'ici une guilde était un serveur Discord (`guilds.id` = snowflake) et un
-- overlay portait seul son mode (`NORMAL` / `INVITE_READ_ONLY`). Cette
-- migration est purement additive :
--
--   * `guilds.standalone = 1` marque une guilde créée hors Discord (panel
--     serveur). Pour `standalone = 0`, l'identifiant reste le snowflake du
--     serveur Discord : toutes les lignes existantes, et celles que les flux
--     Discord créent encore à la volée, gardent donc leur sens sans réécriture.
--     Une guilde indépendante peut être liée à un serveur Discord par
--     `discord_guild_id` ;
--   * `guild_members` porte le profil (pseudo, avatar) et le rôle
--     (`ADMIN` / `MEMBER` / `GUEST`) ; un overlay appartient à un membre
--     (`overlay_clients.member_id`). `session_mode` reste écrit pour la parité
--     du protocole WS v2 ;
--   * `guild_invites` : codes d'invitation porteurs d'un rôle, à usages
--     limités. Les codes émis depuis Discord restent dans `pairing_codes` ;
--   * `meme_board_items.created_by_member_id` : auteur, pour le droit
--     « modifier ses propres mèmes ».
--
-- Reprise de l'existant : chaque overlay déjà appairé devient un membre
-- (même identifiant), `MEMBER` en mode normal, `GUEST` en lecture seule ; un
-- overlay révoqué donne un membre retiré. Aucune guilde existante n'a d'admin
-- après migration : le premier code admin s'émet depuis le panel serveur.

ALTER TABLE guilds ADD COLUMN standalone INTEGER NOT NULL DEFAULT 0
    CHECK (standalone IN (0, 1));
ALTER TABLE guilds ADD COLUMN discord_guild_id TEXT;
ALTER TABLE guilds ADD COLUMN guests_can_view_board INTEGER NOT NULL DEFAULT 1
    CHECK (guests_can_view_board IN (0, 1));
ALTER TABLE guilds ADD COLUMN created_at INTEGER;

CREATE UNIQUE INDEX idx_guilds_discord_guild_id
    ON guilds (discord_guild_id)
    WHERE discord_guild_id IS NOT NULL;

-- Membres d'une guilde.
CREATE TABLE guild_members (
    id           TEXT    PRIMARY KEY,
    guild_id     TEXT    NOT NULL,
    display_name TEXT    NOT NULL,
    avatar       TEXT,
    role         TEXT    NOT NULL CHECK (role IN ('ADMIN', 'MEMBER', 'GUEST')),
    created_at   INTEGER NOT NULL,
    removed_at   INTEGER
) STRICT;

CREATE INDEX idx_guild_members_guild ON guild_members (guild_id, created_at);

ALTER TABLE overlay_clients ADD COLUMN member_id TEXT REFERENCES guild_members (id);

CREATE INDEX idx_overlay_clients_member ON overlay_clients (member_id);

INSERT INTO guild_members (id, guild_id, display_name, avatar, role, created_at, removed_at)
SELECT
    id,
    guild_id,
    COALESCE(NULLIF(TRIM(default_author_name), ''), label),
    default_author_image,
    CASE session_mode WHEN 'INVITE_READ_ONLY' THEN 'GUEST' ELSE 'MEMBER' END,
    created_at,
    revoked_at
FROM overlay_clients;

UPDATE overlay_clients SET member_id = id;

-- Invitations (codes porteurs d'un rôle).
CREATE TABLE guild_invites (
    code                 TEXT    PRIMARY KEY,
    guild_id             TEXT    NOT NULL,
    role                 TEXT    NOT NULL CHECK (role IN ('ADMIN', 'MEMBER', 'GUEST')),
    member_id            TEXT    REFERENCES guild_members (id) ON DELETE CASCADE,
    max_uses             INTEGER CHECK (max_uses IS NULL OR max_uses > 0),
    use_count            INTEGER NOT NULL DEFAULT 0,
    created_by_member_id TEXT,
    expires_at           INTEGER,
    revoked_at           INTEGER,
    created_at           INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_guild_invites_guild ON guild_invites (guild_id, created_at);

ALTER TABLE meme_board_items ADD COLUMN created_by_member_id TEXT;
