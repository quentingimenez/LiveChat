PRAGMA foreign_keys=OFF;

CREATE TABLE "new_PairingCode" (
    "code" TEXT NOT NULL PRIMARY KEY,
    "guildId" TEXT NOT NULL,
    "mode" TEXT NOT NULL DEFAULT 'NORMAL',
    "createdByDiscordUserId" TEXT NOT NULL,
    "authorName" TEXT,
    "authorImage" TEXT,
    "expiresAt" DATETIME,
    "usedAt" DATETIME,
    "createdAt" DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO "new_PairingCode" (
    "code",
    "guildId",
    "mode",
    "createdByDiscordUserId",
    "authorName",
    "authorImage",
    "expiresAt",
    "usedAt",
    "createdAt"
)
SELECT
    "code",
    "guildId",
    'NORMAL',
    "createdByDiscordUserId",
    "authorName",
    "authorImage",
    "expiresAt",
    "usedAt",
    "createdAt"
FROM "PairingCode";

DROP TABLE "PairingCode";
ALTER TABLE "new_PairingCode" RENAME TO "PairingCode";

CREATE INDEX "PairingCode_guildId_expiresAt_idx" ON "PairingCode"("guildId", "expiresAt");
CREATE INDEX "PairingCode_expiresAt_idx" ON "PairingCode"("expiresAt");

PRAGMA foreign_keys=ON;
