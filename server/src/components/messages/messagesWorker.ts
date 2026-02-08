import { addMilliseconds, addSeconds } from 'date-fns';
import { OVERLAY_SOCKET_EVENTS, type OverlayPlayPayload } from '@livechat/overlay-protocol';
import { MediaAssetStatus, PlaybackJobStatus } from '../../services/prisma/prismaEnums';

const buildOverlayPlayPayload = (params: {
  job: {
    id: string;
    text: string | null;
    showText: boolean;
    durationSec: number;
  };
  mediaAsset:
    | {
        id: string;
        mime: string;
        kind: string;
        durationSec: number | null;
        isVertical: boolean;
      }
    | null;
}): OverlayPlayPayload => {
  const media = params.mediaAsset
    ? {
        assetId: params.mediaAsset.id,
        url: `${env.API_URL}/overlay/media/${params.mediaAsset.id}`,
        mime: params.mediaAsset.mime,
        kind: params.mediaAsset.kind.toLowerCase() as 'image' | 'audio' | 'video',
        durationSec: params.mediaAsset.durationSec,
        isVertical: params.mediaAsset.isVertical,
      }
    : null;

  return {
    jobId: params.job.id,
    media,
    text: {
      value: params.job.text || '',
      enabled: params.job.showText,
    },
    durationSec: params.job.durationSec,
  };
};

export const executeMessagesWorker = async (fastify: FastifyCustomInstance) => {
  const nextJob = await prisma.playbackJob.findFirst({
    where: {
      status: PlaybackJobStatus.PENDING,
      executionDate: {
        lte: new Date(),
      },
    },
    orderBy: {
      executionDate: 'asc',
    },
  });

  if (nextJob === null) {
    logger.debug(`[SOCKET] No new message`);
    return;
  }

  const guild = await prisma.guild.findFirst({
    where: {
      id: nextJob.guildId,
      busyUntil: {
        gte: new Date(),
      },
    },
  });

  if (guild) {
    await prisma.playbackJob.update({
      where: {
        id: nextJob.id,
      },
      data: {
        executionDate: addMilliseconds(new Date(), 250),
      },
    });
    return;
  }

  let busyUntil = addSeconds(new Date(), nextJob.durationSec);
  busyUntil = addMilliseconds(busyUntil, 250);

  await prisma.guild.upsert({
    where: {
      id: nextJob.guildId,
    },
    create: {
      id: nextJob.guildId,
      busyUntil,
    },
    update: {
      busyUntil,
    },
  });

  let mediaAsset: {
    id: string;
    mime: string;
    kind: string;
    durationSec: number | null;
    isVertical: boolean;
  } | null = null;

  if (nextJob.mediaAssetId) {
    const asset = await prisma.mediaAsset.findFirst({
      where: {
        id: nextJob.mediaAssetId,
      },
    });

    if (!asset || asset.status !== MediaAssetStatus.READY) {
      await prisma.playbackJob.update({
        where: {
          id: nextJob.id,
        },
        data: {
          status: PlaybackJobStatus.FAILED,
          finishedAt: new Date(),
        },
      });

      logger.warn(`[SOCKET] Job ${nextJob.id} skipped because media asset is unavailable`);
      return 100;
    }

    mediaAsset = {
      id: asset.id,
      mime: asset.mime,
      kind: asset.kind,
      durationSec: asset.durationSec,
      isVertical: asset.isVertical,
    };
  }

  await prisma.playbackJob.update({
    where: {
      id: nextJob.id,
    },
    data: {
      status: PlaybackJobStatus.PLAYING,
      startedAt: new Date(),
    },
  });

  const payload = buildOverlayPlayPayload({
    job: {
      id: nextJob.id,
      text: nextJob.text,
      showText: nextJob.showText,
      durationSec: nextJob.durationSec,
    },
    mediaAsset,
  });

  fastify.io.to(`overlay-guild-${nextJob.guildId}`).emit(OVERLAY_SOCKET_EVENTS.PLAY, payload);

  await prisma.playbackJob.update({
    where: {
      id: nextJob.id,
    },
    data: {
      status: PlaybackJobStatus.DONE,
      finishedAt: new Date(),
    },
  });

  logger.debug(`[SOCKET] Playback job ${nextJob.id} sent to guild ${nextJob.guildId}`);

  return nextJob.durationSec * 1000 || 5000;
};

export const loadMessagesWorker = async (fastify: FastifyCustomInstance) => {
  await executeMessagesWorker(fastify);

  setTimeout(() => {
    loadMessagesWorker(fastify);
  }, 100);
};
