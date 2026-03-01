import { hashOverlayToken } from '../services/overlayAuth';
import { createPlaybackJob } from '../services/playbackJobs';
import { MediaAssetStatus } from '../services/prisma/prismaEnums';
import {
  OVERLAY_SOCKET_EVENTS,
  type OverlayErrorPayload,
  type OverlayHeartbeatPayload,
  type OverlayMemeTriggerPayload,
  type OverlayPlaybackStatePayload,
  type OverlayStopPayload,
} from '@livechat/overlay-protocol';
import { getPlaybackScheduler, MEME_JOB_PRIORITY } from '../services/playbackScheduler';
import { executeManualStopForGuild } from '../services/manualStop';

const toNonEmptyString = (value: unknown): string | undefined => {
  if (typeof value !== 'string') {
    return undefined;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : undefined;
};

const getTokenFromSocketHandshake = (socket) => {
  const authToken = socket.handshake?.auth?.token;

  if (typeof authToken === 'string' && authToken.trim()) {
    return authToken.trim();
  }

  const queryToken = socket.handshake?.query?.token;

  if (typeof queryToken === 'string' && queryToken.trim()) {
    return queryToken.trim();
  }

  return null;
};

const listConnectedOverlayPeers = async (fastify: FastifyCustomInstance, guildId: string) => {
  const roomName = `overlay-guild-${guildId}`;
  const sockets = await fastify.io.in(roomName).fetchSockets();
  const peersByClientId = new Map<string, { clientId: string; label: string }>();

  for (const roomSocket of sockets) {
    const clientId =
      typeof roomSocket.data?.overlayClientId === 'string' && roomSocket.data.overlayClientId.trim() !== ''
        ? roomSocket.data.overlayClientId.trim()
        : '';
    const label =
      typeof roomSocket.data?.overlayClientLabel === 'string' && roomSocket.data.overlayClientLabel.trim() !== ''
        ? roomSocket.data.overlayClientLabel.trim()
        : 'unknown-device';

    if (!clientId || peersByClientId.has(clientId)) {
      continue;
    }

    peersByClientId.set(clientId, {
      clientId,
      label,
    });
  }

  return Array.from(peersByClientId.values()).sort((a, b) => {
    const labelOrder = a.label.localeCompare(b.label, undefined, { sensitivity: 'base' });
    if (labelOrder !== 0) {
      return labelOrder;
    }

    return a.clientId.localeCompare(b.clientId, undefined, { sensitivity: 'base' });
  });
};

const broadcastOverlayPeers = async (fastify: FastifyCustomInstance, guildId: string) => {
  const peers = await listConnectedOverlayPeers(fastify, guildId);
  const roomName = `overlay-guild-${guildId}`;

  fastify.io.to(roomName).emit(OVERLAY_SOCKET_EVENTS.PEERS, {
    guildId,
    peers,
  });

  logger.debug(`[OVERLAY] Peers updated for guild ${guildId} (count: ${peers.length})`);
};

export const loadSocket = (fastify: FastifyCustomInstance) => {
  logger.info(`[Socket] Socket loaded`);
  const playbackScheduler = getPlaybackScheduler();

  if (!playbackScheduler) {
    throw new Error('playback_scheduler_not_initialized');
  }

  fastify.io.use(async (socket, next) => {
    try {
      const token = getTokenFromSocketHandshake(socket);

      if (!token) {
        logger.warn({ socketId: socket.id }, '[OVERLAY] Socket auth failed: missing token');
        next(new Error('missing_token'));
        return;
      }

      const tokenHash = hashOverlayToken(token);

      const client = await prisma.overlayClient.findFirst({
        where: {
          tokenHash,
          revokedAt: null,
        },
      });

      if (!client) {
        logger.warn({ socketId: socket.id }, '[OVERLAY] Socket auth failed: invalid token');
        next(new Error('invalid_token'));
        return;
      }

      socket.data.guildId = client.guildId;
      socket.data.overlayClientId = client.id;
      socket.data.overlayClientLabel = client.label;
      socket.data.overlayAuthorName = toNonEmptyString((client as { defaultAuthorName?: unknown }).defaultAuthorName);
      socket.data.overlayAuthorImage = toNonEmptyString(
        (client as { defaultAuthorImage?: unknown }).defaultAuthorImage,
      );

      await prisma.overlayClient.update({
        where: {
          id: client.id,
        },
        data: {
          lastSeenAt: new Date(),
        },
      });

      next();
    } catch (error) {
      logger.error({ err: error, socketId: socket.id }, '[OVERLAY] Socket auth error');
      next(new Error('auth_error'));
    }
  });

  fastify.io.on('connection', (socket) => {
    const guildId = socket.data.guildId;

    if (!guildId) {
      socket.disconnect(true);
      return;
    }

    socket.join(`overlay-guild-${guildId}`);
    const roomName = `overlay-guild-${guildId}`;
    const roomSize = fastify.io.sockets.adapter.rooms.get(roomName)?.size ?? 0;
    const clientLabel = socket.data.overlayClientLabel || 'unknown-device';
    logger.info(
      `[OVERLAY] Connected: ${clientLabel} (clientId: ${socket.data.overlayClientId}, socket: ${socket.id}, guild: ${guildId}, roomSize: ${roomSize})`,
    );

    void playbackScheduler.onJobEnqueued(guildId).catch((error) => {
      logger.warn({ err: error, guildId }, '[PLAYBACK] Failed to run scheduler after overlay connect');
    });

    void broadcastOverlayPeers(fastify, guildId).catch((error) => {
      logger.warn({ err: error, guildId }, '[OVERLAY] Failed to broadcast peers after connect');
    });

    socket.on(OVERLAY_SOCKET_EVENTS.HEARTBEAT, async (payload: OverlayHeartbeatPayload) => {
      logger.debug(
        `Heartbeat from ${socket.data.overlayClientLabel || 'unknown-device'} (${payload?.clientId || socket.data.overlayClientId}, socket: ${socket.id})`,
      );

      await prisma.overlayClient.updateMany({
        where: {
          id: socket.data.overlayClientId,
          guildId: socket.data.guildId,
          revokedAt: null,
        },
        data: {
          lastSeenAt: new Date(),
        },
      });
    });

    socket.on(OVERLAY_SOCKET_EVENTS.ERROR, (payload: OverlayErrorPayload) => {
      logger.warn(
        `[OVERLAY] Error from ${socket.data.overlayClientLabel || 'unknown-device'} (${socket.data.overlayClientId}) on job ${
          payload?.jobId
        }: ${payload?.code} ${payload?.message}`,
      );
    });

    socket.on(OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE, async (payload: OverlayPlaybackStatePayload) => {
      const playbackJobId = typeof payload?.jobId === 'string' && payload.jobId.trim() ? payload.jobId.trim() : null;
      const playbackState =
        payload?.state === 'paused' || payload?.state === 'ended' || payload?.state === 'playing'
          ? payload.state
          : 'playing';
      const remainingMs =
        typeof payload?.remainingMs === 'number' && Number.isFinite(payload.remainingMs)
          ? Math.max(0, Math.min(payload.remainingMs, 24 * 60 * 60 * 1000))
          : null;

      logger.debug(
        `[OVERLAY] Playback state received from ${socket.data.overlayClientLabel || 'unknown-device'} (${
          socket.data.overlayClientId
        }, guild: ${guildId}, jobId: ${playbackJobId || 'unknown'}, state: ${playbackState}, remainingMs: ${
          remainingMs === null ? 'null' : remainingMs
        })`,
      );

      await playbackScheduler.onPlaybackState({
        guildId,
        jobId: playbackJobId,
        state: playbackState,
        remainingMs,
      });
    });

    socket.on(OVERLAY_SOCKET_EVENTS.STOP, async (payload: OverlayStopPayload) => {
      const stopJobId = typeof payload?.jobId === 'string' && payload.jobId.trim() ? payload.jobId.trim() : 'unknown';

      if (stopJobId === 'manual-stop') {
        await executeManualStopForGuild(fastify, guildId, {
          logLabel: 'Stop command',
        });
      } else {
        await playbackScheduler.onPlaybackStopped({
          guildId,
          jobId: stopJobId,
        });
      }

      logger.info(
        `[OVERLAY] Stop received from ${socket.data.overlayClientLabel || 'unknown-device'} (${
          socket.data.overlayClientId
        }, guild: ${guildId}, jobId: ${stopJobId})`,
      );
    });

    socket.on(OVERLAY_SOCKET_EVENTS.MEME_TRIGGER, async (payload: OverlayMemeTriggerPayload) => {
      const rawItemId = typeof payload?.itemId === 'string' ? payload.itemId.trim() : '';
      const triggerKind = payload?.trigger === 'ui' ? 'ui' : 'shortcut';

      if (!rawItemId) {
        logger.warn(
          `[OVERLAY] Meme trigger ignored from ${socket.data.overlayClientLabel || 'unknown-device'} (${
            socket.data.overlayClientId
          }, guild: ${guildId}): missing itemId`,
        );
        return;
      }

      const item = await prisma.memeBoardItem.findFirst({
        where: {
          id: rawItemId,
          guildId,
        },
        include: {
          mediaAsset: true,
        },
      });

      if (!item || !item.mediaAsset || item.mediaAsset.status !== MediaAssetStatus.READY) {
        logger.warn(
          `[OVERLAY] Meme trigger ignored from ${socket.data.overlayClientLabel || 'unknown-device'} (${
            socket.data.overlayClientId
          }, guild: ${guildId}, itemId: ${rawItemId}): unavailable media`,
        );
        return;
      }

      const itemAuthorName =
        typeof item.createdByName === 'string' && item.createdByName.trim() !== '' ? item.createdByName.trim() : null;
      const itemMessage =
        typeof item.message === 'string' && item.message.trim() !== '' ? item.message.trim().slice(0, 500) : null;
      const triggerAuthorName = toNonEmptyString(socket.data.overlayAuthorName) || itemAuthorName;
      const triggerAuthorImage = toNonEmptyString(socket.data.overlayAuthorImage);

      const job = await createPlaybackJob({
        guildId,
        mediaAsset: item.mediaAsset,
        text: itemMessage,
        showText: !!itemMessage,
        authorName: triggerAuthorName,
        authorImage: triggerAuthorImage,
        priority: MEME_JOB_PRIORITY,
        skipScheduleNotify: true,
        source: `overlay_meme_trigger_${triggerKind}`,
      });

      await playbackScheduler.preemptWithJob({
        guildId,
        preemptingJobId: job.id,
      });

      logger.info(
        `[OVERLAY] Meme trigger accepted from ${socket.data.overlayClientLabel || 'unknown-device'} (${
          socket.data.overlayClientId
        }, guild: ${guildId}, itemId: ${rawItemId}, trigger: ${triggerKind}, jobId: ${job.id})`,
      );
    });

    socket.on('disconnecting', () => {
      logger.info(
        `[OVERLAY] Disconnected: ${socket.data.overlayClientLabel || 'unknown-device'} (clientId: ${
          socket.data.overlayClientId
        }, socket: ${socket.id}, guild: ${guildId})`,
      );
    });

    socket.on('disconnect', async (reason) => {
      const roomNameAfterDisconnect = `overlay-guild-${guildId}`;
      const remainingClients = fastify.io.sockets.adapter.rooms.get(roomNameAfterDisconnect)?.size ?? 0;
      await broadcastOverlayPeers(fastify, guildId).catch((error) => {
        logger.warn({ err: error, guildId }, '[OVERLAY] Failed to broadcast peers after disconnect');
      });

      if (remainingClients > 0) {
        return;
      }

      await prisma.guild.upsert({
        where: {
          id: guildId,
        },
        create: {
          id: guildId,
          busyUntil: null,
        },
        update: {
          busyUntil: null,
        },
      });

      logger.info(
        `[OVERLAY] Cleared busyUntil after disconnect (guild: ${guildId}, reason: ${reason}, remainingClients: ${remainingClients})`,
      );
    });
  });
};
