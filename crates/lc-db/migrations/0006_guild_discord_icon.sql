-- Discord guild icon hash, refreshed by the guild_create and guild_update
-- gateway handlers. NULL when the linked server has no icon.
ALTER TABLE guilds ADD COLUMN discord_icon_hash TEXT;
