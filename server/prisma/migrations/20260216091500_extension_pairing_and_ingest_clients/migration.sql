-- Add author metadata to pairing codes so extension pairing can pre-fill sender identity
ALTER TABLE "PairingCode" ADD COLUMN "authorName" TEXT;
ALTER TABLE "PairingCode" ADD COLUMN "authorImage" TEXT;

-- Store per-extension ingest clients with revocable, unique tokens
CREATE TABLE "IngestClient" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "label" TEXT NOT NULL,
    "tokenHash" TEXT NOT NULL,
    "defaultAuthorName" TEXT NOT NULL,
    "defaultAuthorImage" TEXT,
    "createdByDiscordUserId" TEXT NOT NULL,
    "lastSeenAt" DATETIME,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "revokedAt" DATETIME
);

CREATE UNIQUE INDEX "IngestClient_tokenHash_key" ON "IngestClient"("tokenHash");
CREATE INDEX "IngestClient_guildId_idx" ON "IngestClient"("guildId");
