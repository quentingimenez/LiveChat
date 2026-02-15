import { addMilliseconds } from 'date-fns';
import { hashOverlayToken } from '../services/overlayAuth';
import { PlaybackJobStatus } from '../services/prisma/prismaEnums';
import {
  OVERLAY_SOCKET_EVENTS,
  type OverlayErrorPayload,
  type OverlayHeartbeatPayload,
  type OverlayPlaybackStatePayload,
  type OverlayStopPayload,
} from '@livechat/overlay-protocol';

const MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS = 5_000;

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

export const loadSocket = (fastify: FastifyCustomInstance) => {
  logger.info(`[Socket] Socket loaded`);

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

      if (playbackState === 'ended') {
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

        const where = playbackJobId
          ? {
              guildId,
              id: playbackJobId,
              status: PlaybackJobStatus.PLAYING,
              finishedAt: null,
            }
          : {
              guildId,
              status: PlaybackJobStatus.PLAYING,
              finishedAt: null,
            };

        const releasedJobs = await prisma.playbackJob.updateMany({
          where,
          data: {
            status: PlaybackJobStatus.DONE,
            finishedAt: new Date(),
          },
        });

        logger.info(
          `[OVERLAY] Playback ended from ${socket.data.overlayClientLabel || 'unknown-device'} (${socket.data.overlayClientId}, guild: ${guildId}, jobId: ${
            playbackJobId || 'unknown'
          }, releasedJobs: ${releasedJobs.count})`,
        );
        return;
      }

      if (playbackState === 'paused' || playbackState === 'playing') {
        const lockMs = Math.max(remainingMs ?? 0, MIN_ACTIVE_PLAYBACK_BUSY_LOCK_MS);
        const busyUntil = addMilliseconds(new Date(), lockMs + 250);

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

        return;
      }
    });

    socket.on(OVERLAY_SOCKET_EVENTS.STOP, async (payload: OverlayStopPayload) => {
      const stopJobId = typeof payload?.jobId === 'string' && payload.jobId.trim() ? payload.jobId.trim() : 'unknown';

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

      const releasedJobs = await prisma.playbackJob.updateMany({
        where: {
          guildId,
          status: PlaybackJobStatus.PLAYING,
          finishedAt: null,
        },
        data: {
          status: PlaybackJobStatus.DONE,
          finishedAt: new Date(),
        },
      });

      logger.info(
        `[OVERLAY] Stop received from ${socket.data.overlayClientLabel || 'unknown-device'} (${
          socket.data.overlayClientId
        }, guild: ${guildId}, jobId: ${stopJobId}, releasedJobs: ${releasedJobs.count})`,
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
