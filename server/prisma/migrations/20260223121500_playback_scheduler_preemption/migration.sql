-- Add scheduler/preemption metadata to playback jobs.
ALTER TABLE "PlaybackJob" ADD COLUMN "priority" INTEGER NOT NULL DEFAULT 0;
ALTER TABLE "PlaybackJob" ADD COLUMN "resumesAfterJobId" TEXT;
ALTER TABLE "PlaybackJob" ADD COLUMN "resumeOffsetSec" INTEGER DEFAULT 0;
ALTER TABLE "PlaybackJob" ADD COLUMN "remainingMsSnapshot" INTEGER;
ALTER TABLE "PlaybackJob" ADD COLUMN "lastPlaybackStateAt" DATETIME;

CREATE INDEX IF NOT EXISTS "PlaybackJob_guildId_status_priority_submissionDate_idx"
  ON "PlaybackJob"("guildId", "status", "priority", "submissionDate");

CREATE INDEX IF NOT EXISTS "PlaybackJob_guildId_status_resumesAfterJobId_idx"
  ON "PlaybackJob"("guildId", "status", "resumesAfterJobId");
