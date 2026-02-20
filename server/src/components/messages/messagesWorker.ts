import { addMilliseconds, addSeconds } from 'date-fns';
import { OVERLAY_SOCKET_EVENTS, type OverlayPlayPayload } from '@livechat/overlay-protocol';
import { MediaAssetStatus, PlaybackJobStatus } from '../../services/prisma/prismaEnums';
import { decodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';

const STALE_PLAYING_RELEASE_GRACE_MS = 10_000;
const STALE_PLAYING_FALLBACK_LOCK_MS = 5_000;

const toOptionalPositiveInt = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }

  const normalized = Math.floor(value);
  return normalized > 0 ? normalized : null;
};

const getMediaStartOffsetSec = (richPayload: ReturnType<typeof decodeRichOverlayPayload>): number | null => {
  if (!richPayload || richPayload.type !== 'media') {
    return null;
  }

  return toOptionalPositiveInt(richPayload.startOffsetSec);
};

const getEffectivePlaybackDurationSec = (params: {
  durationSec: number;
  mediaAsset: {
    kind: string;
  } | null;
  startOffsetSec: number | null;
}): number => {
  if (params.startOffsetSec === null || !params.mediaAsset) {
    return params.durationSec;
  }

  if (params.mediaAsset.kind === 'IMAGE') {
    return params.durationSec;
  }

  return Math.max(1, params.durationSec - params.startOffsetSec);
};

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
  const richPayload = params.richPayload;
  const isTweetRichPayload = !!richPayload && richPayload.type === 'tweet';
  const isMediaRichPayload = !!richPayload && richPayload.type === 'media';
  const tweetCard = isTweetRichPayload ? richPayload.tweetCard : null;
  const tweetCaption = isTweetRichPayload ? (richPayload.caption || '').trim() : '';
  const mediaCaption = isMediaRichPayload ? (richPayload.caption || '').trim() : '';
  const mediaStartOffsetSec = isMediaRichPayload ? toOptionalPositiveInt(richPayload.startOffsetSec) : null;
  const hasTweetCard = !!tweetCard;
  const textValue = hasTweetCard ? tweetCaption : isMediaRichPayload ? mediaCaption : params.job.text || '';
  const textEnabled = hasTweetCard ? tweetCaption.length > 0 : params.job.showText;
  const authorEnabled = hasTweetCard ? false : !!params.job.authorName;

  const mediaUrl = params.mediaAsset ? new URL(`${env.API_URL}/overlay/media/${params.mediaAsset.id}`) : null;

  if (mediaUrl && mediaStartOffsetSec !== null) {
    mediaUrl.searchParams.set('startOffsetSec', `${mediaStartOffsetSec}`);
    mediaUrl.hash = `t=${mediaStartOffsetSec}`;
  }

  const media = params.mediaAsset
    ? {
        assetId: params.mediaAsset.id,
        url: mediaUrl ? mediaUrl.toString() : `${env.API_URL}/overlay/media/${params.mediaAsset.id}`,
        mime: params.mediaAsset.mime,
        kind: params.mediaAsset.kind.toLowerCase() as 'image' | 'audio' | 'video',
        durationSec: params.mediaAsset.durationSec,
        isVertical: params.mediaAsset.isVertical,
        startOffsetSec: mediaStartOffsetSec,
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

  const now = new Date();
  const [guild, activePlayingJob] = await Promise.all([
    prisma.guild.findFirst({
      where: {
        id: nextJob.guildId,
      },
      select: {
        id: true,
        busyUntil: true,
      },
    }),
    prisma.playbackJob.findFirst({
      where: {
        guildId: nextJob.guildId,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      select: {
        id: true,
        startedAt: true,
        durationSec: true,
      },
    }),
  ]);

  let currentPlayingJob = activePlayingJob;

  if (currentPlayingJob) {
    const busyUntilMs = guild?.busyUntil?.getTime() || now.getTime();
    const remainingMs = Math.max(0, busyUntilMs - now.getTime());
    const startedAtMs = currentPlayingJob.startedAt?.getTime() || null;
    const expectedEndAtMs =
      startedAtMs !== null
        ? startedAtMs + Math.max(1, currentPlayingJob.durationSec) * 1000 + STALE_PLAYING_RELEASE_GRACE_MS
        : null;
    const shouldAutoReleaseStale = remainingMs <= 0 && expectedEndAtMs !== null && now.getTime() >= expectedEndAtMs;

    if (shouldAutoReleaseStale) {
      const releasedJobs = await prisma.playbackJob.updateMany({
        where: {
          id: currentPlayingJob.id,
          guildId: nextJob.guildId,
          status: PlaybackJobStatus.PLAYING,
          finishedAt: null,
        },
        data: {
          status: PlaybackJobStatus.DONE,
          finishedAt: now,
        },
      });

      if (releasedJobs.count > 0) {
        await prisma.guild.upsert({
          where: {
            id: nextJob.guildId,
          },
          create: {
            id: nextJob.guildId,
            busyUntil: null,
          },
          update: {
            busyUntil: null,
          },
        });

        logger.warn(
          `[SOCKET] Auto-released stale PLAYING job ${currentPlayingJob.id} in guild ${nextJob.guildId} (durationSec: ${
            currentPlayingJob.durationSec
          }, startedAt: ${currentPlayingJob.startedAt?.toISOString() || 'unknown'})`,
        );

        currentPlayingJob = null;
      }
    }
  }

  if (currentPlayingJob) {
    const busyUntilMs = guild?.busyUntil?.getTime() || now.getTime();
    const remainingMs = Math.max(0, busyUntilMs - now.getTime());
    const postponeMs = Math.max(250, Math.min(5000, remainingMs > 0 ? remainingMs : STALE_PLAYING_FALLBACK_LOCK_MS));

    if (remainingMs <= 0) {
      const nextBusyUntil = addMilliseconds(now, STALE_PLAYING_FALLBACK_LOCK_MS + 250);

      await prisma.guild.upsert({
        where: {
          id: nextJob.guildId,
        },
        create: {
          id: nextJob.guildId,
          busyUntil: nextBusyUntil,
        },
        update: {
          busyUntil: nextBusyUntil,
        },
      });

      logger.warn(
        `[SOCKET] Active PLAYING job ${currentPlayingJob.id} has no valid busy lock in guild ${nextJob.guildId}; extending lock by ${STALE_PLAYING_FALLBACK_LOCK_MS}ms`,
      );
    } else if (remainingMs > 1000) {
      logger.info(
        `[SOCKET] Job ${nextJob.id} deferred for guild ${nextJob.guildId} (remainingMs: ${remainingMs}, nextTryInMs: ${postponeMs}, activeJobId: ${currentPlayingJob.id})`,
      );
    }

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

  if (guild && guild.busyUntil && guild.busyUntil >= now) {
    await prisma.guild.upsert({
      where: {
        id: nextJob.guildId,
      },
      create: {
        id: nextJob.guildId,
        busyUntil: null,
      },
      update: {
        busyUntil: null,
      },
    });

    logger.warn(`[SOCKET] Released stale busy lock for guild ${nextJob.guildId} while scheduling job ${nextJob.id}`);
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

  const richPayload = decodeRichOverlayPayload(nextJob.text);
  const mediaStartOffsetSec = getMediaStartOffsetSec(richPayload);
  const effectiveDurationSec = getEffectivePlaybackDurationSec({
    durationSec: nextJob.durationSec,
    mediaAsset,
    startOffsetSec: mediaStartOffsetSec,
  });

  let busyUntil = addSeconds(new Date(), effectiveDurationSec);
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

  const payload = buildOverlayPlayPayload({
    job: {
      id: nextJob.id,
      text: nextJob.text,
      showText: nextJob.showText,
      authorName: nextJob.authorName,
      authorImage: nextJob.authorImage,
      durationSec: effectiveDurationSec,
    },
    richPayload,
    mediaAsset,
  });
  const queueDelayMs = Math.max(0, Date.now() - nextJob.submissionDate.getTime());
  logger.info(
    `[SOCKET] Dispatching job ${nextJob.id} to guild ${nextJob.guildId} (clients: ${roomSize}, durationSec: ${effectiveDurationSec}, rawDurationSec: ${nextJob.durationSec}, queueDelayMs: ${queueDelayMs}, mediaKind: ${
      mediaAsset?.kind || 'none'
    }, mediaDurationSec: ${mediaAsset?.durationSec ?? 'n/a'}, mediaStartOffsetSec: ${payload.media?.startOffsetSec ?? 'n/a'})`,
  );

  fastify.io.to(roomName).emit(OVERLAY_SOCKET_EVENTS.PLAY, payload);
  return 100;
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
