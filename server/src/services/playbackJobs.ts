import type { MediaAsset } from '@prisma/client';
import { getDurationFromGuildId } from './utils';

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
  const durationSec = await getDurationFromGuildId(
    params.durationSec ?? params.mediaAsset?.durationSec,
    params.guildId,
  );
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

  logger.info(
    {
      source: params.source || 'unknown',
      guildId: params.guildId,
      jobId: job.id,
      hasMedia: !!params.mediaAsset,
      hasText: !!params.text,
      showText: params.showText ?? !!params.text,
      durationSec: job.durationSec,
    },
    '[PLAYBACK] Job created',
  );

  return job;
};
