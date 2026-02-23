import { subHours } from 'date-fns';
import { getDurationFromGuildId } from './utils';
import { PlaybackJobStatus } from './prisma/prismaEnums';
import { decodeRichOverlayPayload } from './messages/richOverlayPayload';
import { notifyPlaybackSchedulerJobEnqueued } from './playbackScheduler';

interface MediaAssetLike {
  id: string;
  durationSec?: number | null;
  kind?: string | null;
}

interface CreatePlaybackJobParams {
  guildId: string;
  mediaAsset?: MediaAssetLike | null;
  text?: string | null;
  authorName?: string | null;
  authorImage?: string | null;
  showText?: boolean;
  durationSec?: number | null;
  priority?: number | null;
  resumesAfterJobId?: string | null;
  resumeOffsetSec?: number | null;
  skipScheduleNotify?: boolean;
  source?: string;
}

const toOptionalPositiveInt = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }

  const normalized = Math.floor(value);
  return normalized > 0 ? normalized : null;
};

const toNonNegativeInt = (value: unknown): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }

  const normalized = Math.floor(value);
  return normalized > 0 ? normalized : 0;
};

const toNormalizedPriority = (value: unknown): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }

  return Math.floor(value);
};

const getMediaStartOffsetSecFromText = (value: string | null | undefined): number | null => {
  const richPayload = decodeRichOverlayPayload(value);
  if (!richPayload || richPayload.type !== 'media') {
    return null;
  }

  return toOptionalPositiveInt(richPayload.startOffsetSec);
};

export const createPlaybackJob = async (params: CreatePlaybackJobParams) => {
  const startedAtMs = Date.now();
  const hasMedia = !!params.mediaAsset;
  const mediaKind = typeof params.mediaAsset?.kind === 'string' ? params.mediaAsset.kind.trim().toUpperCase() : null;
  const isImageMedia = mediaKind === 'IMAGE';
  const requestedResumeOffsetSec = toNonNegativeInt(params.resumeOffsetSec);
  const embeddedStartOffsetSec =
    hasMedia && !isImageMedia && requestedResumeOffsetSec === 0 ? getMediaStartOffsetSecFromText(params.text) : null;
  const durationCandidate =
    params.durationSec ??
    (!isImageMedia ? params.mediaAsset?.durationSec ?? null : null) ??
    (hasMedia ? Math.max(1, env.DEFAULT_DURATION) : null);

  if (hasMedia && !isImageMedia && params.durationSec == null && params.mediaAsset?.durationSec == null) {
    logger.warn(
      {
        source: params.source || 'unknown',
        guildId: params.guildId,
        fallbackDurationSec: Math.max(1, env.DEFAULT_DURATION),
      },
      '[PLAYBACK] Media duration unknown, using fallback default duration',
    );
  }

  const baseDurationSec = await getDurationFromGuildId(durationCandidate, params.guildId);
  const effectiveStartOffsetSec = embeddedStartOffsetSec ?? 0;
  const normalizedDurationSec =
    hasMedia && !isImageMedia && effectiveStartOffsetSec > 0
      ? Math.max(1, baseDurationSec - effectiveStartOffsetSec)
      : baseDurationSec;
  const normalizedResumeOffsetSec = requestedResumeOffsetSec + effectiveStartOffsetSec;
  const normalizedPriority = toNormalizedPriority(params.priority);
  const afterDurationResolveMs = Date.now();
  const now = new Date();
  const job = await prisma.playbackJob.create({
    data: {
      guildId: params.guildId,
      mediaAssetId: params.mediaAsset?.id,
      text: params.text || null,
      showText: params.showText ?? !!params.text,
      authorName: params.authorName || null,
      authorImage: params.authorImage || null,
      durationSec: normalizedDurationSec,
      priority: normalizedPriority,
      resumesAfterJobId: params.resumesAfterJobId || null,
      resumeOffsetSec: normalizedResumeOffsetSec,
      executionDate: now,
      scheduledAt: now,
    },
  });
  const finishedAtMs = Date.now();
  const resolveDurationMs = afterDurationResolveMs - startedAtMs;
  const insertDurationMs = finishedAtMs - afterDurationResolveMs;
  const totalDurationMs = finishedAtMs - startedAtMs;

  if (totalDurationMs >= 500) {
    logger.warn(
      {
        source: params.source || 'unknown',
        guildId: params.guildId,
        jobId: job.id,
        resolveDurationMs,
        insertDurationMs,
        totalDurationMs,
      },
      '[PLAYBACK] Slow job creation',
    );
  }

  logger.info(
    {
      source: params.source || 'unknown',
      guildId: params.guildId,
      jobId: job.id,
      hasMedia,
      hasText: !!params.text,
      showText: params.showText ?? !!params.text,
      durationSec: job.durationSec,
      priority: normalizedPriority,
      resumeOffsetSec: normalizedResumeOffsetSec,
      resumesAfterJobId: params.resumesAfterJobId || null,
    },
    '[PLAYBACK] Job created',
  );

  if (params.skipScheduleNotify !== true) {
    notifyPlaybackSchedulerJobEnqueued(params.guildId);
  }

  return job;
};

const getPlaybackJobRetentionHours = () => {
  return Math.max(1, env.PLAYBACK_JOB_RETENTION_HOURS);
};

export const purgeOldPlaybackJobs = async () => {
  const retentionCutoff = subHours(new Date(), getPlaybackJobRetentionHours());

  const deleteResult = await prisma.playbackJob.deleteMany({
    where: {
      status: {
        in: [PlaybackJobStatus.DONE, PlaybackJobStatus.FAILED],
      },
      finishedAt: {
        lte: retentionCutoff,
      },
    },
  });

  if (deleteResult.count > 0) {
    logger.info(`[PLAYBACK] Purged ${deleteResult.count} playback jobs older than ${getPlaybackJobRetentionHours()}h`);
  }
};

export const startPlaybackJobPurgeWorker = () => {
  setInterval(
    async () => {
      try {
        await purgeOldPlaybackJobs();
      } catch (error) {
        logger.error(error, '[PLAYBACK] Job purge failed');
      }
    },
    10 * 60 * 1000,
  );
};
