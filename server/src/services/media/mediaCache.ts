import fs from 'fs';
import { mkdir, rm } from 'fs/promises';
import path from 'path';
import { addHours, subHours } from 'date-fns';
import { MediaAssetStatus } from '../prisma/prismaEnums';

const getCacheTtlHours = () => {
  return Math.max(1, env.MEDIA_CACHE_TTL_HOURS);
};

export const getMediaStorageDir = () => {
  return path.resolve(env.MEDIA_STORAGE_DIR);
};

export const ensureMediaStorageDir = async () => {
  await mkdir(getMediaStorageDir(), { recursive: true });
};

export const buildMediaOutputBasePath = (sourceHash: string) => {
  return path.join(getMediaStorageDir(), sourceHash);
};

export const getReadyCachedMediaAsset = async (sourceHash: string) => {
  const asset = await prisma.mediaAsset.findUnique({
    where: {
      sourceHash,
    },
  });

  if (!asset) {
    return null;
  }

  if (asset.status !== MediaAssetStatus.READY) {
    return null;
  }

  if (asset.expiresAt <= new Date()) {
    return null;
  }

  if (!fs.existsSync(asset.storagePath)) {
    return null;
  }

  return asset;
};

export const touchMediaAsset = async (assetId: string) => {
  const now = new Date();
  const nextExpiry = addHours(now, getCacheTtlHours());
  const current = await prisma.mediaAsset.findUnique({
    where: {
      id: assetId,
    },
  });

  if (!current) {
    return null;
  }

  const preservedExpiry = current.expiresAt > nextExpiry ? current.expiresAt : nextExpiry;

  return prisma.mediaAsset.update({
    where: {
      id: assetId,
    },
    data: {
      lastAccessedAt: now,
      expiresAt: preservedExpiry,
    },
  });
};

const purgeOneAsset = async (asset: { id: string; storagePath: string }) => {
  try {
    if (asset.storagePath) {
      await rm(asset.storagePath, { force: true });
    }
  } catch (error) {
    logger.warn(`[MEDIA] Unable to remove file from cache: ${asset.storagePath}`);
  }

  await prisma.mediaAsset.delete({
    where: {
      id: asset.id,
    },
  });
};

export const purgeExpiredMediaAssets = async () => {
  const now = new Date();

  const expiredAssets = await prisma.mediaAsset.findMany({
    where: {
      OR: [
        {
          expiresAt: {
            lte: now,
          },
        },
        {
          status: {
            in: [MediaAssetStatus.FAILED, MediaAssetStatus.PROCESSING],
          },
          createdAt: {
            lte: subHours(now, 2),
          },
        },
      ],
    },
    select: {
      id: true,
      storagePath: true,
    },
  });

  for (const asset of expiredAssets) {
    await purgeOneAsset(asset);
  }

  if (expiredAssets.length > 0) {
    logger.info(`[MEDIA] Purged ${expiredAssets.length} expired media assets`);
  }
};

export const startMediaCachePurgeWorker = () => {
  setInterval(async () => {
    try {
      await purgeExpiredMediaAssets();
    } catch (error) {
      logger.error(error, '[MEDIA] Cache purge failed');
    }
  }, 60 * 1000);
};
