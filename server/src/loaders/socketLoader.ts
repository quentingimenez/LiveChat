import { hashOverlayToken } from '../services/overlayAuth';
import { PlaybackJobStatus } from '../services/prisma/prismaEnums';
import {
  OVERLAY_SOCKET_EVENTS,
  type OverlayErrorPayload,
  type OverlayHeartbeatPayload,
  type OverlayStopPayload,
} from '@livechat/overlay-protocol';

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
