import { addMilliseconds, addSeconds } from 'date-fns';
import { MediaAssetStatus, PlaybackJobStatus } from '../../services/prisma/prismaEnums';
import { decodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { OVERLAY_SOCKET_EVENTS, type OverlayPlayPayload } from '@livechat/overlay-protocol';

const buildOverlayPlayPayload = (params: {
  job: {
    id: string;
    text: string | null;
    showText: boolean;
    authorName: string | null;
    authorImage: string | null;
    durationSec: number;
  };
  richPayload: ReturnType<typeof decodeRichOverlayPayload>;
  mediaAsset: {
    id: string;
    mime: string;
    kind: string;
    durationSec: number | null;
    isVertical: boolean;
  } | null;
}): OverlayPlayPayload => {
  const tweetCard = params.richPayload?.type === 'tweet' ? params.richPayload.tweetCard : null;
  const tweetCaption = params.richPayload?.type === 'tweet' ? (params.richPayload.caption || '').trim() : '';
  const hasTweetCard = !!tweetCard;
  const textValue = hasTweetCard ? tweetCaption : params.job.text || '';
  const textEnabled = hasTweetCard ? tweetCaption.length > 0 : params.job.showText;
  const authorEnabled = hasTweetCard ? false : !!params.job.authorName;

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
      value: textValue,
      enabled: textEnabled,
    },
    author: {
      name: params.job.authorName || '',
      image: params.job.authorImage || null,
      enabled: authorEnabled,
    },
    tweetCard: tweetCard
      ? {
          ...tweetCard,
          caption: tweetCaption || null,
        }
      : null,
    durationSec: params.job.durationSec,
  };
};

export const executeMessagesWorker = async (fastify: FastifyCustomInstance) => {
  const lookupStartedAtMs = Date.now();
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
  const lookupDurationMs = Date.now() - lookupStartedAtMs;
  if (lookupDurationMs >= 500) {
    logger.warn(
      {
        lookupDurationMs,
      },
      '[SOCKET] Slow playback queue lookup',
    );
  }

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
    const now = new Date();
    const busyUntilMs = guild.busyUntil?.getTime() || now.getTime();
    const remainingMs = Math.max(0, busyUntilMs - now.getTime());
    const postponeMs = Math.max(250, Math.min(5000, remainingMs));

    await prisma.playbackJob.update({
      where: {
        id: nextJob.id,
      },
      data: {
        executionDate: addMilliseconds(now, postponeMs),
      },
    });
    return;
  }

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

  const roomName = `overlay-guild-${nextJob.guildId}`;
  const roomSize = fastify.io.sockets.adapter.rooms.get(roomName)?.size ?? 0;

  if (roomSize === 0) {
    await prisma.playbackJob.update({
      where: {
        id: nextJob.id,
      },
      data: {
        status: PlaybackJobStatus.FAILED,
        finishedAt: new Date(),
      },
    });

    logger.warn(`[SOCKET] Job ${nextJob.id} skipped because no overlay is connected for guild ${nextJob.guildId}`);
    return 100;
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

  await prisma.playbackJob.update({
    where: {
      id: nextJob.id,
    },
    data: {
      status: PlaybackJobStatus.PLAYING,
      startedAt: new Date(),
    },
  });

  const richPayload = decodeRichOverlayPayload(nextJob.text);
  const payload = buildOverlayPlayPayload({
    job: {
      id: nextJob.id,
      text: nextJob.text,
      showText: nextJob.showText,
      authorName: nextJob.authorName,
      authorImage: nextJob.authorImage,
      durationSec: nextJob.durationSec,
    },
    richPayload,
    mediaAsset,
  });
  const queueDelayMs = Math.max(0, Date.now() - nextJob.submissionDate.getTime());
  logger.info(
    `[SOCKET] Dispatching job ${nextJob.id} to guild ${nextJob.guildId} (clients: ${roomSize}, durationSec: ${nextJob.durationSec}, queueDelayMs: ${queueDelayMs})`,
  );

  fastify.io.to(roomName).emit(OVERLAY_SOCKET_EVENTS.PLAY, payload);

  await prisma.playbackJob.update({
    where: {
      id: nextJob.id,
    },
    data: {
      status: PlaybackJobStatus.DONE,
      finishedAt: new Date(),
    },
  });

  logger.info(
    `[SOCKET] Playback job ${nextJob.id} marked as DONE for guild ${nextJob.guildId} (durationSec: ${nextJob.durationSec})`,
  );

  return nextJob.durationSec * 1000 || 5000;
};

export const loadMessagesWorker = async (fastify: FastifyCustomInstance) => {
  try {
    await executeMessagesWorker(fastify);
  } catch (error) {
    logger.error({ err: error }, '[SOCKET] Messages worker iteration failed');
  }

  setTimeout(() => {
    void loadMessagesWorker(fastify);
  }, 100);
};
