import { Routes, type APIUser as User } from 'discord.js';

/**
 * Fetches current user info from Discord API
 */
export const fetchDiscordUserInfo = async (discordUserId: string): Promise<User | null> => {
  try {
    if (!global.discordRest) {
      logger.warn('[DISCORD_SYNC] Discord REST client not available');
      return null;
    }

    const user = (await global.discordRest.get(Routes.user(discordUserId))) as User;
    return user;
  } catch (error) {
    if ((error as any)?.status === 404) {
      logger.debug(`[DISCORD_SYNC] User ${discordUserId} not found on Discord`);
    } else {
      logger.error(error, `[DISCORD_SYNC] Failed to fetch user ${discordUserId}`);
    }
    return null;
  }
};

/**
 * Generates Discord avatar URL from user data
 */
export const getDiscordAvatarUrl = (user: User): string | null => {
  if (!user.avatar) {
    return null;
  }

  const extension = user.avatar.startsWith('a_') ? 'gif' : 'png';
  return `https://cdn.discordapp.com/avatars/${user.id}/${user.avatar}.${extension}?size=256`;
};

/**
 * Gets the display name for a Discord user (preferring global_name over username)
 */
export const getDiscordDisplayName = (user: User): string => {
  return user.global_name || user.username;
};

/**
 * Syncs a single Discord user's profile info across all records
 * Updates IngestClient, PairingCode, and OverlayClient records
 */
export const syncDiscordUserProfile = async (discordUserId: string): Promise<boolean> => {
  try {
    const userInfo = await fetchDiscordUserInfo(discordUserId);
    if (!userInfo) {
      return false;
    }

    const displayName = getDiscordDisplayName(userInfo);
    const avatarUrl = getDiscordAvatarUrl(userInfo);

    // Update IngestClient records
    const ingestUpdateResult = await prisma.ingestClient.updateMany({
      where: {
        createdByDiscordUserId: discordUserId,
        revokedAt: null, // Only update active clients
      },
      data: {
        defaultAuthorName: displayName,
        defaultAuthorImage: avatarUrl,
      },
    });

    // Update PairingCode records (non-expired ones)
    const pairingUpdateResult = await prisma.pairingCode.updateMany({
      where: {
        createdByDiscordUserId: discordUserId,
        OR: [{ expiresAt: null }, { expiresAt: { gt: new Date() } }],
      },
      data: {
        authorName: displayName,
        authorImage: avatarUrl,
      },
    });

    // Update OverlayClient records with default author
    const overlayUpdateResult = await prisma.overlayClient.updateMany({
      where: {
        createdByDiscordUserId: discordUserId,
        revokedAt: null,
        defaultAuthorName: { not: null }, // Only update if they have a default author set
      },
      data: {
        defaultAuthorName: displayName,
        defaultAuthorImage: avatarUrl,
      },
    });

    logger.info(
      `[DISCORD_SYNC] Synced user ${discordUserId}: ${ingestUpdateResult.count} ingest clients, ${pairingUpdateResult.count} pairing codes, ${overlayUpdateResult.count} overlay clients`,
    );

    return true;
  } catch (error) {
    logger.error(error, `[DISCORD_SYNC] Failed to sync user ${discordUserId}`);
    return false;
  }
};

/**
 * Syncs all active Discord users' profiles
 * Only syncs users who have activity in the last 30 days
 */
export const syncAllActiveDiscordUsers = async (): Promise<void> => {
  try {
    const thirtyDaysAgo = new Date();
    thirtyDaysAgo.setDate(thirtyDaysAgo.getDate() - 30);

    // Get unique Discord user IDs from active records
    const [ingestUsers, pairingUsers, overlayUsers] = await Promise.all([
      prisma.ingestClient.findMany({
        where: {
          revokedAt: null,
          OR: [{ lastSeenAt: { gte: thirtyDaysAgo } }, { createdAt: { gte: thirtyDaysAgo } }],
        },
        select: { createdByDiscordUserId: true },
        distinct: ['createdByDiscordUserId'],
      }),
      prisma.pairingCode.findMany({
        where: {
          OR: [{ expiresAt: null }, { expiresAt: { gt: new Date() } }],
          createdAt: { gte: thirtyDaysAgo },
        },
        select: { createdByDiscordUserId: true },
        distinct: ['createdByDiscordUserId'],
      }),
      prisma.overlayClient.findMany({
        where: {
          revokedAt: null,
          createdByDiscordUserId: { not: null },
          OR: [{ lastSeenAt: { gte: thirtyDaysAgo } }, { createdAt: { gte: thirtyDaysAgo } }],
        },
        select: { createdByDiscordUserId: true },
        distinct: ['createdByDiscordUserId'],
      }),
    ]);

    // Combine and deduplicate user IDs
    const uniqueUserIds = new Set<string>();
    ingestUsers.forEach((u) => uniqueUserIds.add(u.createdByDiscordUserId));
    pairingUsers.forEach((u) => uniqueUserIds.add(u.createdByDiscordUserId));
    overlayUsers.forEach((u) => u.createdByDiscordUserId && uniqueUserIds.add(u.createdByDiscordUserId));

    logger.info(`[DISCORD_SYNC] Starting sync for ${uniqueUserIds.size} active users`);

    let successCount = 0;
    let failureCount = 0;

    // Sync each user with rate limiting (avoid Discord API limits)
    for (const userId of uniqueUserIds) {
      const success = await syncDiscordUserProfile(userId);
      if (success) {
        successCount++;
      } else {
        failureCount++;
      }

      // Rate limit: wait 100ms between requests
      await new Promise((resolve) => setTimeout(resolve, 100));
    }

    logger.info(
      `[DISCORD_SYNC] Completed sync: ${successCount} successful, ${failureCount} failed out of ${uniqueUserIds.size} users`,
    );
  } catch (error) {
    logger.error(error, '[DISCORD_SYNC] Failed to sync all users');
  }
};
