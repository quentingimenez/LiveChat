import type { MediaAsset } from '@prisma/client';
import { getDurationFromGuildId } from './utils';

interface CreatePlaybackJobParams {
  guildId: string;
  mediaAsset?: MediaAsset | null;
  text?: string | null;
  authorName?: string | null;
  authorImage?: string | null;
  showText?: boolean;
}

export const createPlaybackJob = async (params: CreatePlaybackJobParams) => {
  const durationSec = await getDurationFromGuildId(params.mediaAsset?.durationSec, params.guildId);

  return prisma.playbackJob.create({
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
};
