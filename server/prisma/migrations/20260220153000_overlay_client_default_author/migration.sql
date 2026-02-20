-- Persist overlay client author profile from pairing metadata so meme triggers
-- can render Discord name + avatar like extension playback.
ALTER TABLE "OverlayClient" ADD COLUMN "defaultAuthorName" TEXT;
ALTER TABLE "OverlayClient" ADD COLUMN "defaultAuthorImage" TEXT;
ALTER TABLE "OverlayClient" ADD COLUMN "createdByDiscordUserId" TEXT;
