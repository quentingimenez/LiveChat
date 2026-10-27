-- Drop legacy queue table
DROP TABLE IF EXISTS "Queue";

-- CreateTable
CREATE TABLE "MediaAsset" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "sourceHash" TEXT NOT NULL,
    "sourceUrl" TEXT NOT NULL,
    "kind" TEXT NOT NULL,
    "mime" TEXT NOT NULL,
    "durationSec" INTEGER,
    "width" INTEGER,
    "height" INTEGER,
    "isVertical" BOOLEAN NOT NULL DEFAULT false,
    "storagePath" TEXT NOT NULL,
    "sizeBytes" INTEGER NOT NULL DEFAULT 0,
    "status" TEXT NOT NULL DEFAULT 'PROCESSING',
    "error" TEXT,
    "lastAccessedAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "expiresAt" DATETIME NOT NULL,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- CreateTable
CREATE TABLE "PlaybackJob" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mediaAssetId" TEXT,
    "text" TEXT,
    "showText" BOOLEAN NOT NULL DEFAULT false,
    "authorName" TEXT,
    "authorImage" TEXT,
    "durationSec" INTEGER NOT NULL,
    "status" TEXT NOT NULL DEFAULT 'PENDING',
    "submissionDate" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "executionDate" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "scheduledAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "startedAt" DATETIME,
    "finishedAt" DATETIME,
    CONSTRAINT "PlaybackJob_mediaAssetId_fkey" FOREIGN KEY ("mediaAssetId") REFERENCES "MediaAsset" ("id") ON DELETE SET NULL ON UPDATE CASCADE
);

-- CreateTable
CREATE TABLE "OverlayClient" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "lastSeenAt" DATETIME,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "revokedAt" DATETIME
);

-- CreateTable
CREATE TABLE "PairingCode" (
    "code" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "createdByDiscordUserId" TEXT NOT NULL,
    "expiresAt" DATETIME NOT NULL,
    "usedAt" DATETIME,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- CreateIndex
CREATE UNIQUE INDEX "MediaAsset_sourceHash_key" ON "MediaAsset"("sourceHash");

-- CreateIndex
CREATE INDEX "MediaAsset_expiresAt_idx" ON "MediaAsset"("expiresAt");

-- CreateIndex
CREATE INDEX "MediaAsset_status_idx" ON "MediaAsset"("status");

-- CreateIndex
CREATE INDEX "PlaybackJob_guildId_executionDate_idx" ON "PlaybackJob"("guildId", "executionDate");

-- CreateIndex
CREATE INDEX "PlaybackJob_status_executionDate_idx" ON "PlaybackJob"("status", "executionDate");

-- CreateIndex
CREATE UNIQUE INDEX "OverlayClient_tokenHash_key" ON "OverlayClient"("tokenHash");

-- CreateIndex
CREATE INDEX "OverlayClient_guildId_idx" ON "OverlayClient"("guildId");

-- CreateIndex
CREATE INDEX "PairingCode_guildId_expiresAt_idx" ON "PairingCode"("guildId", "expiresAt");

-- CreateIndex
CREATE INDEX "PairingCode_expiresAt_idx" ON "PairingCode"("expiresAt");
