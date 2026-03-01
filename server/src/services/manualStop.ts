import { OVERLAY_SOCKET_EVENTS } from '@livechat/overlay-protocol';
import { PlaybackJobStatus } from './prisma/prismaEnums';
import { getPlaybackScheduler } from './playbackScheduler';

type ManualStopOptions = {
  logLabel?: string;
};

export const executeManualStopForGuild = async (
  fastify: FastifyCustomInstance,
  guildId: string,
  options: ManualStopOptions = {},
) => {
  const normalizedGuildId = `${guildId || ''}`.trim();

  if (!normalizedGuildId) {
    return {
      releasedCount: 0,
    };
  }

  fastify.io.to(`overlay-guild-${normalizedGuildId}`).emit(OVERLAY_SOCKET_EVENTS.STOP, { jobId: 'manual-stop' });

  await prisma.guild.upsert({
    where: {
      id: normalizedGuildId,
    },
    create: {
      id: normalizedGuildId,
      busyUntil: null,
    },
    update: {
      busyUntil: null,
    },
  });

  const releasedJobs = await prisma.playbackJob.updateMany({
    where: {
      guildId: normalizedGuildId,
      status: PlaybackJobStatus.PLAYING,
      finishedAt: null,
    },
    data: {
      status: PlaybackJobStatus.DONE,
      finishedAt: new Date(),
    },
  });

  const logLabel = typeof options.logLabel === 'string' && options.logLabel.trim() ? options.logLabel.trim() : 'Stop command';
  logger.info(`[PLAYBACK] ${logLabel} released ${releasedJobs.count} playing job(s) for guild ${normalizedGuildId}`);

  const playbackScheduler = getPlaybackScheduler();
  if (playbackScheduler) {
    await playbackScheduler.onManualStop(normalizedGuildId);
  }

  return {
    releasedCount: releasedJobs.count,
  };
};
