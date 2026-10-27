import { addHours, addYears } from 'date-fns';
import { Prisma } from '@prisma/client';
import { MediaAssetStatus } from './prisma/prismaEnums';
import { MediaIngestionError } from './media/mediaErrors';
import { toNonEmptyString, formatBytesForMessage } from './stringUtils';

const PINNED_EXPIRY_YEARS = 100;
const DEFAULT_LIST_LIMIT = 50;
const MAX_LIST_LIMIT = 200;
const BYTES_PER_MEGABYTE = 1024 * 1024;
const BYTES_PER_GIGABYTE = 1024 * BYTES_PER_MEGABYTE;

const normalizeListLimit = (value?: number | null): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return DEFAULT_LIST_LIMIT;
  }

  return Math.min(MAX_LIST_LIMIT, Math.max(1, Math.floor(value)));
};

const normalizeListOffset = (value?: number | null): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }

  return Math.max(0, Math.floor(value));
};

const getPinnedExpiry = (): Date => {
  return addYears(new Date(), PINNED_EXPIRY_YEARS);
};

const getCacheTtlExpiry = (): Date => {
  return addHours(new Date(), Math.max(1, env.MEDIA_CACHE_TTL_HOURS));
};

const getBoardMaxTotalBytes = () => {
  return Math.max(1, env.MEDIA_BOARD_MAX_TOTAL_MB) * BYTES_PER_MEGABYTE;
};

const toSafeSizeBytes = (value: unknown): number => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    return 0;
  }

  return Math.floor(value);
};


const getGuildBoardTotalSizeBytes = async (guildId: string): Promise<number> => {
  const items = await prisma.memeBoardItem.findMany({
    where: {
      guildId,
    },
    select: {
      mediaAsset: {
        select: {
          sizeBytes: true,
        },
      },
    },
  });

  return items.reduce((accumulator, item) => accumulator + toSafeSizeBytes(item.mediaAsset.sizeBytes), 0);
};

const ensureGuildBoardHasCapacity = async (guildId: string, mediaAssetId: string) => {
  const mediaAsset = await prisma.mediaAsset.findUnique({
    where: {
      id: mediaAssetId,
    },
    select: {
      id: true,
      sizeBytes: true,
      status: true,
    },
  });

  if (!mediaAsset || mediaAsset.status !== MediaAssetStatus.READY) {
    return;
  }

  const requestedBytes = toSafeSizeBytes(mediaAsset.sizeBytes);
  const currentTotalBytes = await getGuildBoardTotalSizeBytes(guildId);
  const maxTotalBytes = getBoardMaxTotalBytes();

  if (currentTotalBytes + requestedBytes <= maxTotalBytes) {
    return;
  }

  throw new MediaIngestionError(
    'BOARD_STORAGE_LIMIT_REACHED',
    `Persistent meme board storage limit reached (${formatBytesForMessage(currentTotalBytes)}/${formatBytesForMessage(maxTotalBytes)}). Requested media: ${formatBytesForMessage(requestedBytes)}. Remove older board media first.`,
    `Guild ${guildId} board limit reached; current=${currentTotalBytes} bytes; requested=${requestedBytes} bytes; max=${maxTotalBytes} bytes; mediaAssetId=${mediaAssetId}`,
  );
};

export const ensurePinnedExpiry = async (mediaAssetId: string) => {
  const pinnedExpiry = getPinnedExpiry();

  await prisma.mediaAsset.updateMany({
    where: {
      id: mediaAssetId,
      status: MediaAssetStatus.READY,
      expiresAt: {
        lt: pinnedExpiry,
      },
    },
    data: {
      expiresAt: pinnedExpiry,
      lastAccessedAt: new Date(),
    },
  });
};

export const releasePinnedExpiryIfUnused = async (mediaAssetId: string) => {
  const remainingReferences = await prisma.memeBoardItem.count({
    where: {
      mediaAssetId,
    },
  });

  if (remainingReferences > 0) {
    return false;
  }

  const releaseExpiry = getCacheTtlExpiry();
  const updated = await prisma.mediaAsset.updateMany({
    where: {
      id: mediaAssetId,
      status: MediaAssetStatus.READY,
    },
    data: {
      expiresAt: releaseExpiry,
      lastAccessedAt: new Date(),
    },
  });

  return updated.count > 0;
};

export const addToMemeBoard = async (params: {
  guildId: string;
  mediaAssetId: string;
  title?: string | null;
  message?: string | null;
  createdByDiscordUserId?: string | null;
  createdByName?: string | null;
}) => {
  const title = toNonEmptyString(params.title)?.slice(0, 240) || null;
  const message = toNonEmptyString(params.message)?.slice(0, 500) || null;
  const createdByDiscordUserId = toNonEmptyString(params.createdByDiscordUserId);
  const createdByName = toNonEmptyString(params.createdByName)?.slice(0, 120) || null;

  const existing = await prisma.memeBoardItem.findUnique({
    where: {
      guildId_mediaAssetId: {
        guildId: params.guildId,
        mediaAssetId: params.mediaAssetId,
      },
    },
    include: {
      mediaAsset: true,
    },
  });

  if (existing) {
    await ensurePinnedExpiry(params.mediaAssetId);
    return {
      created: false,
      item: existing,
    };
  }

  await ensureGuildBoardHasCapacity(params.guildId, params.mediaAssetId);
  await ensurePinnedExpiry(params.mediaAssetId);

  try {
    const item = await prisma.memeBoardItem.create({
      data: {
        guildId: params.guildId,
        mediaAssetId: params.mediaAssetId,
        title,
        message,
        createdByDiscordUserId,
        createdByName,
      },
      include: {
        mediaAsset: true,
      },
    });

    return {
      created: true,
      item,
    };
  } catch (error) {
    if (!(error instanceof Prisma.PrismaClientKnownRequestError) || error.code !== 'P2002') {
      throw error;
    }

    const item = await prisma.memeBoardItem.findUnique({
      where: {
        guildId_mediaAssetId: {
          guildId: params.guildId,
          mediaAssetId: params.mediaAssetId,
        },
      },
      include: {
        mediaAsset: true,
      },
    });

    await ensurePinnedExpiry(params.mediaAssetId);

    return {
      created: false,
      item,
    };
  }
};

export const listMemeBoardItems = async (params: {
  guildId: string;
  q?: string | null;
  limit?: number | null;
  offset?: number | null;
}) => {
  const limit = normalizeListLimit(params.limit);
  const offset = normalizeListOffset(params.offset);
  const q = toNonEmptyString(params.q)?.slice(0, 240) || null;

  const where = {
    guildId: params.guildId,
    ...(q
      ? {
          OR: [
            {
              title: {
                contains: q,
              },
            },
            {
              message: {
                contains: q,
              },
            },
            {
              createdByName: {
                contains: q,
              },
            },
            {
              mediaAsset: {
                sourceUrl: {
                  contains: q,
                },
              },
            },
          ],
        }
      : {}),
  };

  const [total, items] = await Promise.all([
    prisma.memeBoardItem.count({
      where,
    }),
    prisma.memeBoardItem.findMany({
      where,
      orderBy: {
        createdAt: 'desc',
      },
      include: {
        mediaAsset: true,
      },
      skip: offset,
      take: limit,
    }),
  ]);

  return {
    total,
    limit,
    offset,
    items,
  };
};

export const removeMemeBoardItem = async (params: { guildId: string; itemId: string }) => {
  const existing = await prisma.memeBoardItem.findFirst({
    where: {
      id: params.itemId,
      guildId: params.guildId,
    },
    select: {
      id: true,
      mediaAssetId: true,
    },
  });

  if (!existing) {
    return {
      deleted: false,
      releasedPinnedExpiry: false,
    };
  }

  await prisma.memeBoardItem.delete({
    where: {
      id: existing.id,
    },
  });

  const releasedPinnedExpiry = await releasePinnedExpiryIfUnused(existing.mediaAssetId);

  return {
    deleted: true,
    releasedPinnedExpiry,
  };
};

export const updateMemeBoardItem = async (params: {
  guildId: string;
  itemId: string;
  title?: string | null | undefined;
  message?: string | null | undefined;
}) => {
  const existing = await prisma.memeBoardItem.findFirst({
    where: {
      id: params.itemId,
      guildId: params.guildId,
    },
    select: {
      id: true,
    },
  });

  if (!existing) {
    return null;
  }

  const data: {
    title?: string | null;
    message?: string | null;
  } = {};

  if (params.title !== undefined) {
    data.title = toNonEmptyString(params.title)?.slice(0, 240) || null;
  }

  if (params.message !== undefined) {
    data.message = toNonEmptyString(params.message)?.slice(0, 500) || null;
  }

  if (Object.keys(data).length === 0) {
    return prisma.memeBoardItem.findFirst({
      where: {
        id: existing.id,
      },
      include: {
        mediaAsset: true,
      },
    });
  }

  return prisma.memeBoardItem.update({
    where: {
      id: existing.id,
    },
    data,
    include: {
      mediaAsset: true,
    },
  });
};
