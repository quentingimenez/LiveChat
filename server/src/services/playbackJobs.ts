import type { MediaAsset } from '@prisma/client';
import { subHours } from 'date-fns';
import { getDurationFromGuildId } from './utils';
import { PlaybackJobStatus } from './prisma/prismaEnums';

interface CreatePlaybackJobParams {
  guildId: string;
  mediaAsset?: MediaAsset | null;
  text?: string | null;
  authorName?: string | null;
  authorImage?: string | null;
  showText?: boolean;
  durationSec?: number | null;
  source?: string;
}

export const createPlaybackJob = async (params: CreatePlaybackJobParams) => {
  const startedAtMs = Date.now();
  const hasMedia = !!params.mediaAsset;
  const durationCandidate =
    params.durationSec ?? params.mediaAsset?.durationSec ?? (hasMedia ? Math.max(1, env.DEFAULT_DURATION) : null);

  if (hasMedia && params.durationSec == null && params.mediaAsset?.durationSec == null) {
    logger.warn(
      {
        source: params.source || 'unknown',
        guildId: params.guildId,
        fallbackDurationSec: Math.max(1, env.DEFAULT_DURATION),
      },
      '[PLAYBACK] Media duration unknown, using fallback default duration',
    );
  }

  const durationSec = await getDurationFromGuildId(durationCandidate, params.guildId);
  const afterDurationResolveMs = Date.now();
  const job = await prisma.playbackJob.create({
    data: {
      guildId: params.guildId,
      mediaAssetId: params.mediaAsset?.id,
      text: params.text || null,
      showText: params.showText ?? !!params.text,
      authorName: params.authorName || null,
      authorImage: params.authorImage || null,
      durationSec,
      executionDate: new Date(),
      scheduledAt: new Date(),
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
    },
    '[PLAYBACK] Job created',
  );

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
