-- Improve lookup performance for overlay media authorization and old-job purge scans
CREATE INDEX IF NOT EXISTS "PlaybackJob_guildId_mediaAssetId_idx" ON "PlaybackJob"("guildId", "mediaAssetId");
CREATE INDEX IF NOT EXISTS "PlaybackJob_status_finishedAt_idx" ON "PlaybackJob"("status", "finishedAt");
