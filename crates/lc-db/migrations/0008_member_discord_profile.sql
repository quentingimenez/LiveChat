-- Migration 0008 — members linked to their Discord account.
--
-- A member who paired with their own `/pair-code` (normal mode) is linked to
-- that Discord account (`discord_user_id`): their name and avatar follow
-- Discord until they choose their own in « Mon profil » (`profile_custom`).
-- `discord_synced_at` throttles the reads of the Discord API.
ALTER TABLE guild_members ADD COLUMN discord_user_id TEXT;
ALTER TABLE guild_members ADD COLUMN profile_custom INTEGER NOT NULL DEFAULT 0
    CHECK (profile_custom IN (0, 1));
ALTER TABLE guild_members ADD COLUMN discord_synced_at INTEGER;

-- Existing members keep the profile they have: nothing tells whether it came
-- from Discord or was typed. « Mon profil » switches a linked member back to
-- Discord.
UPDATE guild_members SET profile_custom = 1;

-- Link existing members through the Discord author of the normal-mode code
-- that paired their oldest overlay (read-only invitations are handed to
-- someone else, and `member:` codes belong to the browser extension).
UPDATE guild_members SET discord_user_id = (
    SELECT c.created_by_discord_user_id
    FROM overlay_clients AS c
    WHERE c.member_id = guild_members.id
      AND c.session_mode = 'NORMAL'
      AND c.created_by_discord_user_id GLOB '[0-9]*'
      AND c.created_by_discord_user_id NOT GLOB '*[^0-9]*'
    ORDER BY c.created_at ASC, c.id ASC
    LIMIT 1
);

-- One active member per Discord account and guild: the oldest keeps the link.
UPDATE guild_members SET discord_user_id = NULL
WHERE discord_user_id IS NOT NULL
  AND removed_at IS NULL
  AND EXISTS (
      SELECT 1 FROM guild_members AS older
      WHERE older.guild_id = guild_members.guild_id
        AND older.discord_user_id = guild_members.discord_user_id
        AND older.removed_at IS NULL
        AND (older.created_at < guild_members.created_at
             OR (older.created_at = guild_members.created_at
                 AND older.id < guild_members.id))
  );

CREATE UNIQUE INDEX idx_guild_members_discord_user
    ON guild_members (guild_id, discord_user_id)
    WHERE discord_user_id IS NOT NULL AND removed_at IS NULL;
