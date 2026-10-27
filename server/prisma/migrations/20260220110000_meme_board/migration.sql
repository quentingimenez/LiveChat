-- CreateTable
CREATE TABLE "MemeBoardItem" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mediaAssetId" TEXT NOT NULL,
    "title" TEXT,
    "createdByDiscordUserId" TEXT,
    "createdByName" TEXT,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT "MemeBoardItem_mediaAssetId_fkey" FOREIGN KEY ("mediaAssetId") REFERENCES "MediaAsset" ("id") ON DELETE CASCADE ON UPDATE CASCADE
);

-- CreateIndex
CREATE UNIQUE INDEX "MemeBoardItem_guildId_mediaAssetId_key" ON "MemeBoardItem"("guildId", "mediaAssetId");

-- CreateIndex
CREATE INDEX "MemeBoardItem_guildId_createdAt_idx" ON "MemeBoardItem"("guildId", "createdAt");

-- CreateIndex
CREATE INDEX "MemeBoardItem_mediaAssetId_idx" ON "MemeBoardItem"("mediaAssetId");
