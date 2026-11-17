-- Migration 0015 — guild bans.
--
-- Removing a member used to be the only sanction: the person could get a new
-- `/pair-code` from Discord and come back. A ban keeps the Discord account
-- (when the member had linked one) out of the guild: no new code, no code
-- redeemed, no bot command. `member_id` has no foreign key: the member row
-- may be purged with its guild while the ban history is deleted with it.
CREATE TABLE guild_bans (
    id              TEXT    PRIMARY KEY,
    guild_id        TEXT    NOT NULL,
    member_id       TEXT,
    discord_user_id TEXT,
    display_name    TEXT    NOT NULL,
    avatar          TEXT,
    banned_by       TEXT,
    created_at      INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_guild_bans_guild ON guild_bans (guild_id, created_at);

CREATE UNIQUE INDEX idx_guild_bans_discord_user
    ON guild_bans (guild_id, discord_user_id)
    WHERE discord_user_id IS NOT NULL;
