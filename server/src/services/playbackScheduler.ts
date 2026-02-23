import { addMilliseconds } from 'date-fns';
import { OVERLAY_SOCKET_EVENTS, type OverlayPlayPayload } from '@livechat/overlay-protocol';
import { decodeRichOverlayPayload } from './messages/richOverlayPayload';
import { MediaAssetStatus, PlaybackJobStatus } from './prisma/prismaEnums';

const PLAYBACK_LOCK_PADDING_MS = 250;
const PLAYBACK_STALE_RELEASE_GRACE_MS = 10_000;
const MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS = 5_000;
const PLAYBACK_STATE_MAX_AGE_MS = 15_000;
const GUILD_RUN_MAX_ITERATIONS = 25;

export const MEME_JOB_PRIORITY = 100;

interface RunGuildOptions {
  reason: string;
  justFinishedJobId?: string | null;
  preferredJobId?: string | null;
}

type DispatchOutcome = 'dispatched' | 'retry' | 'idle';

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const toOptionalPositiveInt = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }

  const normalized = Math.floor(value);
  return normalized > 0 ? normalized : null;
};

const toPlaybackDurationSec = (value: unknown): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 1;
  }

  return Math.max(1, Math.floor(value));
};

const getMediaStartOffsetSecFromRichPayload = (richPayload: ReturnType<typeof decodeRichOverlayPayload>): number | null => {
  if (!richPayload || richPayload.type !== 'media') {
    return null;
  }

  return toOptionalPositiveInt(richPayload.startOffsetSec);
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
  startOffsetSec: number | null;
}): OverlayPlayPayload => {
  const richPayload = params.richPayload;
  const isTweetRichPayload = !!richPayload && richPayload.type === 'tweet';
  const isMediaRichPayload = !!richPayload && richPayload.type === 'media';
  const tweetCard = isTweetRichPayload ? richPayload.tweetCard : null;
  const tweetCaption = isTweetRichPayload ? (richPayload.caption || '').trim() : '';
  const mediaCaption = isMediaRichPayload ? (richPayload.caption || '').trim() : '';
  const hasTweetCard = !!tweetCard;
  const textValue = hasTweetCard ? tweetCaption : isMediaRichPayload ? mediaCaption : params.job.text || '';
  const textEnabled = hasTweetCard ? tweetCaption.length > 0 : params.job.showText;
  const authorEnabled = hasTweetCard ? false : !!params.job.authorName;

  const mediaUrl = params.mediaAsset ? new URL(`${env.API_URL}/overlay/media/${params.mediaAsset.id}`) : null;

  if (mediaUrl && params.startOffsetSec !== null) {
    mediaUrl.searchParams.set('startOffsetSec', `${params.startOffsetSec}`);
    mediaUrl.hash = `t=${params.startOffsetSec}`;
  }

  const media = params.mediaAsset
    ? {
        assetId: params.mediaAsset.id,
        url: mediaUrl ? mediaUrl.toString() : `${env.API_URL}/overlay/media/${params.mediaAsset.id}`,
        mime: params.mediaAsset.mime,
        kind: params.mediaAsset.kind.toLowerCase() as 'image' | 'audio' | 'video',
        durationSec: params.mediaAsset.durationSec,
        isVertical: params.mediaAsset.isVertical,
        startOffsetSec: params.startOffsetSec,
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

class PlaybackScheduler {
  private guildTasks = new Map<string, Promise<void>>();
  private wakeTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private watchdogTimers = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(private readonly fastify: FastifyCustomInstance) {}

  public async bootstrap() {
    const activeJobs = await prisma.playbackJob.findMany({
      where: {
        status: {
          in: [PlaybackJobStatus.PENDING, PlaybackJobStatus.PLAYING],
        },
        finishedAt: null,
      },
      select: {
        guildId: true,
      },
    });

    const uniqueGuildIds = Array.from(
      new Set(
        activeJobs
          .map((job) => `${job.guildId || ''}`.trim())
          .filter((guildId): guildId is string => guildId.length > 0),
      ),
    ) as string[];

    for (const guildId of uniqueGuildIds) {
      await this.enqueueGuildRun(guildId, {
        reason: 'bootstrap',
      });
    }
  }

  public async onJobEnqueued(guildId: string) {
    const normalizedGuildId = toNonEmptyString(guildId);

    if (!normalizedGuildId) {
      return;
    }

    await this.enqueueGuildRun(normalizedGuildId, {
      reason: 'job-enqueued',
    });
  }

  public async onManualStop(guildId: string) {
    const normalizedGuildId = toNonEmptyString(guildId);

    if (!normalizedGuildId) {
      return;
    }

    await this.enqueueGuildRun(normalizedGuildId, {
      reason: 'manual-stop',
    });
  }

  public async onPlaybackState(params: {
    guildId: string;
    jobId?: string | null;
    state: 'playing' | 'paused' | 'ended';
    remainingMs: number | null;
  }) {
    const normalizedGuildId = toNonEmptyString(params.guildId);

    if (!normalizedGuildId) {
      return;
    }

    const normalizedJobId = toNonEmptyString(params.jobId);
    const playbackState = params.state;
    const remainingMs =
      typeof params.remainingMs === 'number' && Number.isFinite(params.remainingMs)
        ? Math.max(0, Math.min(Math.floor(params.remainingMs), 24 * 60 * 60 * 1000))
        : null;

    if (playbackState === 'ended') {
      await this.onPlaybackEnded({
        guildId: normalizedGuildId,
        jobId: normalizedJobId,
      });
      return;
    }

    await this.enqueueGuildTask(normalizedGuildId, async () => {
      const now = new Date();
      const lockMs = Math.max(remainingMs ?? 0, MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS);
      const busyUntil = addMilliseconds(now, lockMs + PLAYBACK_LOCK_PADDING_MS);

      await this.setGuildBusyUntil(normalizedGuildId, busyUntil);

      const where = normalizedJobId
        ? {
            guildId: normalizedGuildId,
            id: normalizedJobId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          }
        : {
            guildId: normalizedGuildId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          };

      await prisma.playbackJob.updateMany({
        where,
        data: {
          remainingMsSnapshot: remainingMs,
          lastPlaybackStateAt: now,
        },
      });

      if (normalizedJobId) {
        await this.scheduleWatchdogForJob(normalizedGuildId, normalizedJobId);
      }
    });
  }

  public async onPlaybackStopped(params: {
    guildId: string;
    jobId?: string | null;
  }) {
    const normalizedGuildId = toNonEmptyString(params.guildId);

    if (!normalizedGuildId) {
      return;
    }

    const normalizedJobId = toNonEmptyString(params.jobId);

    await this.enqueueGuildTask(normalizedGuildId, async () => {
      const shouldTargetSingleJob =
        !!normalizedJobId && normalizedJobId !== 'manual-stop' && normalizedJobId !== 'unknown';
      const where = shouldTargetSingleJob
        ? {
            guildId: normalizedGuildId,
            id: normalizedJobId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          }
        : {
            guildId: normalizedGuildId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          };

      const releasedJobs = await prisma.playbackJob.updateMany({
        where,
        data: {
          status: PlaybackJobStatus.DONE,
          finishedAt: new Date(),
          remainingMsSnapshot: null,
          lastPlaybackStateAt: null,
        },
      });

      if (!shouldTargetSingleJob || releasedJobs.count > 0) {
        await this.clearGuildBusyLock(normalizedGuildId);
      }

      logger.info(
        `[PLAYBACK] Stop released ${releasedJobs.count} playing job(s) (guild: ${normalizedGuildId}, targetJobId: ${
          normalizedJobId || 'all'
        })`,
      );

      await this.runGuild(normalizedGuildId, {
        reason: 'playback-stop',
        justFinishedJobId: shouldTargetSingleJob && releasedJobs.count > 0 ? normalizedJobId : null,
      });
    });
  }

  public async onPlaybackEnded(params: {
    guildId: string;
    jobId?: string | null;
  }) {
    const normalizedGuildId = toNonEmptyString(params.guildId);

    if (!normalizedGuildId) {
      return;
    }

    const normalizedJobId = toNonEmptyString(params.jobId);

    await this.enqueueGuildTask(normalizedGuildId, async () => {
      const shouldTargetSingleJob = !!normalizedJobId && normalizedJobId !== 'unknown';
      const where = shouldTargetSingleJob
        ? {
            guildId: normalizedGuildId,
            id: normalizedJobId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          }
        : {
            guildId: normalizedGuildId,
            status: PlaybackJobStatus.PLAYING,
            finishedAt: null,
          };

      const releasedJobs = await prisma.playbackJob.updateMany({
        where,
        data: {
          status: PlaybackJobStatus.DONE,
          finishedAt: new Date(),
          remainingMsSnapshot: null,
          lastPlaybackStateAt: null,
        },
      });

      if (!shouldTargetSingleJob || releasedJobs.count > 0) {
        await this.clearGuildBusyLock(normalizedGuildId);
      }

      logger.info(
        `[PLAYBACK] Ended released ${releasedJobs.count} playing job(s) (guild: ${normalizedGuildId}, targetJobId: ${
          normalizedJobId || 'all'
        })`,
      );

      await this.runGuild(normalizedGuildId, {
        reason: 'playback-ended',
        justFinishedJobId: shouldTargetSingleJob && releasedJobs.count > 0 ? normalizedJobId : null,
      });
    });
  }

  public async preemptWithJob(params: {
    guildId: string;
    preemptingJobId: string;
  }) {
    const normalizedGuildId = toNonEmptyString(params.guildId);
    const normalizedPreemptingJobId = toNonEmptyString(params.preemptingJobId);

    if (!normalizedGuildId || !normalizedPreemptingJobId) {
      return;
    }

    await this.enqueueGuildTask(normalizedGuildId, async () => {
      const preemptingJob = await prisma.playbackJob.findFirst({
        where: {
          guildId: normalizedGuildId,
          id: normalizedPreemptingJobId,
          status: PlaybackJobStatus.PENDING,
          finishedAt: null,
        },
        select: {
          id: true,
        },
      });

      if (!preemptingJob) {
        return;
      }

      const activePlayingJob = await this.findActivePlayingJob(normalizedGuildId);
      let justFinishedJobId: string | null = null;

      if (activePlayingJob) {
        justFinishedJobId = await this.suspendPlayingJobForPreemption({
          guildId: normalizedGuildId,
          activePlayingJob,
          preemptingJobId: normalizedPreemptingJobId,
        });
      }

      await this.runGuild(normalizedGuildId, {
        reason: 'meme-preempt',
        preferredJobId: normalizedPreemptingJobId,
        justFinishedJobId,
      });
    });
  }

  private enqueueGuildTask(guildId: string, task: () => Promise<void>) {
    const previousTask = this.guildTasks.get(guildId) || Promise.resolve();

    const nextTask = previousTask
      .catch(() => undefined)
      .then(async () => {
        try {
          await task();
        } catch (error) {
          logger.error({ err: error, guildId }, '[PLAYBACK] Scheduler guild task failed');
        }
      });

    const trackedTask = nextTask.finally(() => {
      if (this.guildTasks.get(guildId) === trackedTask) {
        this.guildTasks.delete(guildId);
      }
    });

    this.guildTasks.set(guildId, trackedTask);
    return trackedTask;
  }

  private enqueueGuildRun(guildId: string, options: RunGuildOptions) {
    return this.enqueueGuildTask(guildId, async () => {
      await this.runGuild(guildId, options);
    });
  }

  private async runGuild(guildId: string, options: RunGuildOptions) {
    this.clearWakeTimer(guildId);

    let justFinishedJobId = toNonEmptyString(options.justFinishedJobId);
    let preferredJobId = toNonEmptyString(options.preferredJobId);

    for (let iteration = 0; iteration < GUILD_RUN_MAX_ITERATIONS; iteration += 1) {
      const now = new Date();
      const activePlayingJob = await this.findActivePlayingJob(guildId);

      if (activePlayingJob) {
        const staleReleaseJobId = await this.releaseStalePlayingJob(guildId, activePlayingJob, now);

        if (staleReleaseJobId) {
          justFinishedJobId = staleReleaseJobId;
          preferredJobId = null;
          continue;
        }

        this.scheduleWatchdogTimer(guildId, {
          jobId: `${activePlayingJob.id}`,
          startedAt: activePlayingJob.startedAt,
          durationSec: toPlaybackDurationSec(activePlayingJob.durationSec),
        });
        await this.recomputeRootPendingExecutionDates(guildId, this.getExpectedJobEndAtMs(activePlayingJob, now.getTime()));
        return;
      }

      this.clearWatchdogTimer(guildId);
      await this.clearGuildBusyLock(guildId);

      const nextJob = await this.findNextPendingJob({
        guildId,
        now,
        justFinishedJobId,
        preferredJobId,
      });

      if (!nextJob) {
        await this.recomputeRootPendingExecutionDates(guildId, now.getTime());
        await this.scheduleNextWakeTimer(guildId);
        return;
      }

      const dispatchOutcome = await this.dispatchPendingJob(guildId, nextJob);

      justFinishedJobId = null;
      preferredJobId = null;

      if (dispatchOutcome === 'dispatched' || dispatchOutcome === 'idle') {
        return;
      }
    }

    logger.error(
      `[PLAYBACK] Scheduler exceeded max loop iterations (guild: ${guildId}, reason: ${options.reason}, preferredJobId: ${
        preferredJobId || 'none'
      }, justFinishedJobId: ${justFinishedJobId || 'none'})`,
    );
  }

  private async scheduleWatchdogForJob(guildId: string, jobId: string) {
    const activePlayingJob = await prisma.playbackJob.findFirst({
      where: {
        guildId,
        id: jobId,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      select: {
        id: true,
        startedAt: true,
        durationSec: true,
      },
    });

    if (!activePlayingJob) {
      return;
    }

    this.scheduleWatchdogTimer(guildId, {
      jobId,
      startedAt: activePlayingJob.startedAt,
      durationSec: toPlaybackDurationSec(activePlayingJob.durationSec),
    });
  }

  private scheduleWatchdogTimer(
    guildId: string,
    params: {
      jobId: string;
      startedAt: Date | null;
      durationSec: number;
    },
  ) {
    this.clearWatchdogTimer(guildId);

    const startedAtMs = params.startedAt ? params.startedAt.getTime() : Date.now();
    const expectedReleaseAtMs =
      startedAtMs + params.durationSec * 1000 + PLAYBACK_LOCK_PADDING_MS + PLAYBACK_STALE_RELEASE_GRACE_MS;
    const delayMs = Math.max(250, expectedReleaseAtMs - Date.now());

    const watchdogTimer = setTimeout(() => {
      void this.enqueueGuildRun(guildId, {
        reason: 'watchdog',
      });
    }, delayMs);

    this.watchdogTimers.set(guildId, watchdogTimer);
  }

  private clearWatchdogTimer(guildId: string) {
    const timer = this.watchdogTimers.get(guildId);

    if (!timer) {
      return;
    }

    clearTimeout(timer);
    this.watchdogTimers.delete(guildId);
  }

  private async scheduleNextWakeTimer(guildId: string) {
    this.clearWakeTimer(guildId);

    const nextRootPendingJob = await prisma.playbackJob.findFirst({
      where: {
        guildId,
        status: PlaybackJobStatus.PENDING,
        finishedAt: null,
        resumesAfterJobId: null,
      },
      orderBy: [
        {
          executionDate: 'asc',
        },
        {
          priority: 'desc',
        },
        {
          submissionDate: 'asc',
        },
        {
          id: 'asc',
        },
      ],
      select: {
        id: true,
        executionDate: true,
      },
    });

    if (!nextRootPendingJob) {
      return;
    }

    const delayMs = Math.max(0, nextRootPendingJob.executionDate.getTime() - Date.now());
    const wakeTimer = setTimeout(() => {
      void this.enqueueGuildRun(guildId, {
        reason: 'wake',
      });
    }, delayMs);

    this.wakeTimers.set(guildId, wakeTimer);
  }

  private clearWakeTimer(guildId: string) {
    const timer = this.wakeTimers.get(guildId);

    if (!timer) {
      return;
    }

    clearTimeout(timer);
    this.wakeTimers.delete(guildId);
  }

  private async releaseStalePlayingJob(guildId: string, activePlayingJob, now: Date): Promise<string | null> {
    const expectedEndAtMs = this.getExpectedJobEndAtMs(activePlayingJob, now.getTime());
    const shouldRelease = now.getTime() >= expectedEndAtMs + PLAYBACK_STALE_RELEASE_GRACE_MS;

    if (!shouldRelease) {
      return null;
    }

    const releasedJobs = await prisma.playbackJob.updateMany({
      where: {
        id: activePlayingJob.id,
        guildId,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      data: {
        status: PlaybackJobStatus.DONE,
        finishedAt: now,
        remainingMsSnapshot: null,
        lastPlaybackStateAt: null,
      },
    });

    if (releasedJobs.count === 0) {
      return null;
    }

    await this.clearGuildBusyLock(guildId);

    logger.warn(
      `[PLAYBACK] Auto-released stale PLAYING job ${activePlayingJob.id} (guild: ${guildId}, durationSec: ${
        activePlayingJob.durationSec
      }, startedAt: ${activePlayingJob.startedAt?.toISOString() || 'unknown'})`,
    );

    return `${activePlayingJob.id}`;
  }

  private async findNextPendingJob(params: {
    guildId: string;
    now: Date;
    justFinishedJobId: string | null;
    preferredJobId: string | null;
  }) {
    const { guildId, now, justFinishedJobId, preferredJobId } = params;

    if (preferredJobId) {
      const preferredJob = await prisma.playbackJob.findFirst({
        where: {
          id: preferredJobId,
          guildId,
          status: PlaybackJobStatus.PENDING,
          finishedAt: null,
        },
      });

      if (preferredJob) {
        return preferredJob;
      }
    }

    if (justFinishedJobId) {
      const resumedJob = await prisma.playbackJob.findFirst({
        where: {
          guildId,
          status: PlaybackJobStatus.PENDING,
          finishedAt: null,
          resumesAfterJobId: justFinishedJobId,
        },
        orderBy: [
          {
            priority: 'desc',
          },
          {
            submissionDate: 'asc',
          },
          {
            id: 'asc',
          },
        ],
      });

      if (resumedJob) {
        return resumedJob;
      }
    }

    const recoveredResumedJob = await this.findRecoveredResumedJob(guildId);
    if (recoveredResumedJob) {
      return recoveredResumedJob;
    }

    return prisma.playbackJob.findFirst({
      where: {
        guildId,
        status: PlaybackJobStatus.PENDING,
        finishedAt: null,
        resumesAfterJobId: null,
        executionDate: {
          lte: now,
        },
      },
      orderBy: [
        {
          priority: 'desc',
        },
        {
          submissionDate: 'asc',
        },
        {
          id: 'asc',
        },
      ],
    });
  }

  private async findRecoveredResumedJob(guildId: string) {
    const pendingResumedJobs = await prisma.playbackJob.findMany({
      where: {
        guildId,
        status: PlaybackJobStatus.PENDING,
        finishedAt: null,
        resumesAfterJobId: {
          not: null,
        },
      },
      orderBy: [
        {
          priority: 'desc',
        },
        {
          submissionDate: 'asc',
        },
        {
          id: 'asc',
        },
      ],
      take: 20,
      select: {
        id: true,
        resumesAfterJobId: true,
      },
    });

    for (const pendingResumedJob of pendingResumedJobs) {
      const resumesAfterJobId = toNonEmptyString(pendingResumedJob.resumesAfterJobId);
      if (!resumesAfterJobId) {
        return prisma.playbackJob.findFirst({
          where: {
            id: pendingResumedJob.id,
            guildId,
            status: PlaybackJobStatus.PENDING,
            finishedAt: null,
          },
        });
      }

      const parentJob = await prisma.playbackJob.findFirst({
        where: {
          guildId,
          id: resumesAfterJobId,
        },
        select: {
          status: true,
        },
      });

      if (!parentJob || (parentJob.status !== PlaybackJobStatus.PENDING && parentJob.status !== PlaybackJobStatus.PLAYING)) {
        return prisma.playbackJob.findFirst({
          where: {
            id: pendingResumedJob.id,
            guildId,
            status: PlaybackJobStatus.PENDING,
            finishedAt: null,
          },
        });
      }
    }

    return null;
  }

  private async dispatchPendingJob(guildId: string, nextJob): Promise<DispatchOutcome> {
    const now = new Date();

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
        await prisma.playbackJob.updateMany({
          where: {
            id: nextJob.id,
            guildId,
            status: PlaybackJobStatus.PENDING,
            finishedAt: null,
          },
          data: {
            status: PlaybackJobStatus.FAILED,
            finishedAt: now,
          },
        });

        logger.warn(`[PLAYBACK] Job ${nextJob.id} failed: media asset unavailable (guild: ${guildId})`);
        return 'retry';
      }

      mediaAsset = {
        id: asset.id,
        mime: asset.mime,
        kind: asset.kind,
        durationSec: asset.durationSec,
        isVertical: asset.isVertical,
      };
    }

    const roomName = `overlay-guild-${guildId}`;
    const roomSize = this.fastify.io.sockets.adapter.rooms.get(roomName)?.size ?? 0;

    if (roomSize === 0) {
      await prisma.playbackJob.updateMany({
        where: {
          id: nextJob.id,
          guildId,
          status: PlaybackJobStatus.PENDING,
          finishedAt: null,
        },
        data: {
          status: PlaybackJobStatus.FAILED,
          finishedAt: now,
        },
      });

      logger.warn(`[PLAYBACK] Job ${nextJob.id} failed: no overlay connected (guild: ${guildId})`);
      return 'retry';
    }

    const richPayload = decodeRichOverlayPayload(nextJob.text);
    const legacyStartOffsetSec = getMediaStartOffsetSecFromRichPayload(richPayload);
    let startOffsetSec = toOptionalPositiveInt(nextJob.resumeOffsetSec) ?? 0;
    let effectiveDurationSec = toPlaybackDurationSec(nextJob.durationSec);

    if (startOffsetSec <= 0 && legacyStartOffsetSec !== null && mediaAsset && mediaAsset.kind !== 'IMAGE') {
      startOffsetSec = legacyStartOffsetSec;
      effectiveDurationSec = Math.max(1, effectiveDurationSec - legacyStartOffsetSec);
    }

    const startedAt = new Date();
    const busyUntil = addMilliseconds(startedAt, effectiveDurationSec * 1000 + PLAYBACK_LOCK_PADDING_MS);

    await this.setGuildBusyUntil(guildId, busyUntil);

    const promotedJob = await prisma.playbackJob.updateMany({
      where: {
        id: nextJob.id,
        guildId,
        status: PlaybackJobStatus.PENDING,
        finishedAt: null,
      },
      data: {
        status: PlaybackJobStatus.PLAYING,
        startedAt,
        finishedAt: null,
        durationSec: effectiveDurationSec,
        resumeOffsetSec: startOffsetSec,
        resumesAfterJobId: null,
        executionDate: startedAt,
        scheduledAt: startedAt,
        remainingMsSnapshot: null,
        lastPlaybackStateAt: null,
      },
    });

    if (promotedJob.count === 0) {
      return 'retry';
    }

    const payload = buildOverlayPlayPayload({
      job: {
        id: nextJob.id,
        text: nextJob.text,
        showText: !!nextJob.showText,
        authorName: nextJob.authorName,
        authorImage: nextJob.authorImage,
        durationSec: effectiveDurationSec,
      },
      richPayload,
      mediaAsset,
      startOffsetSec: startOffsetSec > 0 ? startOffsetSec : null,
    });

    const queueDelayMs = Math.max(0, Date.now() - new Date(nextJob.submissionDate).getTime());
    logger.info(
      `[PLAYBACK] Dispatching job ${nextJob.id} (guild: ${guildId}, clients: ${roomSize}, priority: ${
        nextJob.priority
      }, durationSec: ${effectiveDurationSec}, queueDelayMs: ${queueDelayMs}, mediaKind: ${
        mediaAsset?.kind || 'none'
      }, mediaStartOffsetSec: ${payload.media?.startOffsetSec ?? 'n/a'})`,
    );

    this.fastify.io.to(roomName).emit(OVERLAY_SOCKET_EVENTS.PLAY, payload);
    this.scheduleWatchdogTimer(guildId, {
      jobId: nextJob.id,
      startedAt,
      durationSec: effectiveDurationSec,
    });
    await this.recomputeRootPendingExecutionDates(guildId, startedAt.getTime() + effectiveDurationSec * 1000 + PLAYBACK_LOCK_PADDING_MS);
    return 'dispatched';
  }

  private async suspendPlayingJobForPreemption(params: {
    guildId: string;
    activePlayingJob;
    preemptingJobId: string;
  }): Promise<string | null> {
    const { guildId, activePlayingJob, preemptingJobId } = params;
    const now = new Date();
    const nowMs = now.getTime();
    const remainingMs = this.estimateRemainingMs(activePlayingJob, nowMs);

    if (remainingMs <= 0) {
      const releasedJobs = await prisma.playbackJob.updateMany({
        where: {
          id: activePlayingJob.id,
          guildId,
          status: PlaybackJobStatus.PLAYING,
          finishedAt: null,
        },
        data: {
          status: PlaybackJobStatus.DONE,
          finishedAt: now,
          remainingMsSnapshot: null,
          lastPlaybackStateAt: null,
        },
      });

      if (releasedJobs.count > 0) {
        await this.clearGuildBusyLock(guildId);
        return `${activePlayingJob.id}`;
      }

      return null;
    }

    const remainingSec = Math.max(1, Math.ceil(remainingMs / 1000));
    const elapsedSec = Math.max(0, toPlaybackDurationSec(activePlayingJob.durationSec) - remainingSec);
    const currentResumeOffsetSec = toOptionalPositiveInt(activePlayingJob.resumeOffsetSec) ?? 0;
    const nextResumeOffsetSec = Math.max(0, currentResumeOffsetSec + elapsedSec);

    const suspendedJobs = await prisma.playbackJob.updateMany({
      where: {
        id: activePlayingJob.id,
        guildId,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      data: {
        status: PlaybackJobStatus.PENDING,
        startedAt: null,
        finishedAt: null,
        durationSec: remainingSec,
        resumesAfterJobId: preemptingJobId,
        resumeOffsetSec: nextResumeOffsetSec,
        executionDate: now,
        scheduledAt: now,
        remainingMsSnapshot: null,
        lastPlaybackStateAt: null,
      },
    });

    if (suspendedJobs.count === 0) {
      return null;
    }

    await this.clearGuildBusyLock(guildId);

    this.fastify.io.to(`overlay-guild-${guildId}`).emit(OVERLAY_SOCKET_EVENTS.STOP, {
      jobId: `${activePlayingJob.id}`,
    });

    logger.info(
      `[PLAYBACK] Preempted active job ${activePlayingJob.id} for priority job ${preemptingJobId} (guild: ${guildId}, remainingSec: ${remainingSec}, nextResumeOffsetSec: ${nextResumeOffsetSec})`,
    );

    return null;
  }

  private estimateRemainingMs(activePlayingJob, nowMs: number): number {
    const snapshotMs = toOptionalPositiveInt(activePlayingJob.remainingMsSnapshot);
    const snapshotAtMs =
      activePlayingJob.lastPlaybackStateAt instanceof Date
        ? activePlayingJob.lastPlaybackStateAt.getTime()
        : toNonEmptyString(activePlayingJob.lastPlaybackStateAt)
          ? new Date(activePlayingJob.lastPlaybackStateAt).getTime()
          : null;

    if (snapshotMs !== null && snapshotAtMs !== null && Number.isFinite(snapshotAtMs)) {
      const ageMs = Math.max(0, nowMs - snapshotAtMs);

      if (ageMs <= PLAYBACK_STATE_MAX_AGE_MS) {
        return Math.max(0, snapshotMs - ageMs);
      }
    }

    const startedAtMs =
      activePlayingJob.startedAt instanceof Date
        ? activePlayingJob.startedAt.getTime()
        : toNonEmptyString(activePlayingJob.startedAt)
          ? new Date(activePlayingJob.startedAt).getTime()
          : null;

    if (startedAtMs !== null && Number.isFinite(startedAtMs)) {
      const elapsedMs = Math.max(0, nowMs - startedAtMs);
      return Math.max(0, toPlaybackDurationSec(activePlayingJob.durationSec) * 1000 - elapsedMs);
    }

    return toPlaybackDurationSec(activePlayingJob.durationSec) * 1000;
  }

  private async findActivePlayingJob(guildId: string) {
    return prisma.playbackJob.findFirst({
      where: {
        guildId,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      orderBy: {
        startedAt: 'asc',
      },
    });
  }

  private getExpectedJobEndAtMs(activePlayingJob, fallbackNowMs: number) {
    const startedAtMs =
      activePlayingJob.startedAt instanceof Date
        ? activePlayingJob.startedAt.getTime()
        : toNonEmptyString(activePlayingJob.startedAt)
          ? new Date(activePlayingJob.startedAt).getTime()
          : fallbackNowMs;

    return startedAtMs + toPlaybackDurationSec(activePlayingJob.durationSec) * 1000 + PLAYBACK_LOCK_PADDING_MS;
  }

  private async recomputeRootPendingExecutionDates(guildId: string, anchorMs: number) {
    const rootPendingJobs = await prisma.playbackJob.findMany({
      where: {
        guildId,
        status: PlaybackJobStatus.PENDING,
        finishedAt: null,
        resumesAfterJobId: null,
      },
      orderBy: [
        {
          priority: 'desc',
        },
        {
          submissionDate: 'asc',
        },
        {
          id: 'asc',
        },
      ],
      select: {
        id: true,
        durationSec: true,
        executionDate: true,
      },
    });

    let cursorMs = Math.max(Date.now(), Math.max(0, Math.floor(anchorMs)));

    for (const rootPendingJob of rootPendingJobs) {
      const nextExecutionDate = new Date(cursorMs);
      const currentExecutionDateMs = new Date(rootPendingJob.executionDate).getTime();

      if (currentExecutionDateMs !== nextExecutionDate.getTime()) {
        await prisma.playbackJob.update({
          where: {
            id: rootPendingJob.id,
          },
          data: {
            executionDate: nextExecutionDate,
            scheduledAt: nextExecutionDate,
          },
        });
      }

      cursorMs += toPlaybackDurationSec(rootPendingJob.durationSec) * 1000 + PLAYBACK_LOCK_PADDING_MS;
    }
  }

  private async setGuildBusyUntil(guildId: string, busyUntil: Date | null) {
    await prisma.guild.upsert({
      where: {
        id: guildId,
      },
      create: {
        id: guildId,
        busyUntil,
      },
      update: {
        busyUntil,
      },
    });
  }

  private async clearGuildBusyLock(guildId: string) {
    await this.setGuildBusyUntil(guildId, null);
  }
}

let playbackSchedulerSingleton: PlaybackScheduler | null = null;

export const initializePlaybackScheduler = async (fastify: FastifyCustomInstance) => {
  if (!playbackSchedulerSingleton) {
    playbackSchedulerSingleton = new PlaybackScheduler(fastify);
  }

  await playbackSchedulerSingleton.bootstrap();
  return playbackSchedulerSingleton;
};

export const getPlaybackScheduler = () => playbackSchedulerSingleton;

export const notifyPlaybackSchedulerJobEnqueued = (guildId: string) => {
  const scheduler = getPlaybackScheduler();
  if (!scheduler) {
    return;
  }

  void scheduler.onJobEnqueued(guildId);
};
