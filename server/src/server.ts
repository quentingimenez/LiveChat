/* eslint-disable @typescript-eslint/no-var-requires */
import 'reflect-metadata';
import Fastify from 'fastify';
import FastifyCORS from '@fastify/cors';
import GracefulServer from '@gquittet/graceful-server';
import unifyFastifyPlugin from 'unify-fastify';
import { loadRoutes } from './loaders/RESTLoader';
import { loadSocket } from './loaders/socketLoader';
import { loadDiscord } from './loaders/DiscordLoader';
import { loadRosetty } from './services/i18n/loader';
import { loadPrismaClient } from './services/prisma/loadPrisma';
import { ensureMediaStorageDir, startMediaCachePurgeWorker } from './services/media/mediaCache';
import { startPlaybackJobPurgeWorker } from './services/playbackJobs';

export const runServer = async () => {
  const logLevel = env.LOG || 'info';
  // LOAD API FRAMEWORK
  //@ts-ignore
  const fastify: FastifyCustomInstance = Fastify({
    logger: { level: logLevel },
    disableRequestLogging: true,
  });

  const logger = fastify.log;
  global.logger = logger;

  await fastify.register(unifyFastifyPlugin, {
    disableDetails: true,
  });

  //LOAD SENDIM
  const gracefulServer = GracefulServer(fastify.server);
  gracefulServer.on(GracefulServer.SHUTTING_DOWN, (err) => {
    if (err) {
      logger.debug(err);
    }
    logger.debug('Server is shutting down');
  });

  try {
    logger.info('[DB] Connected to database');
    await loadPrismaClient();
    logger.info('[BOOT] Prisma connected');
    await ensureMediaStorageDir();
    logger.info('[BOOT] Media storage ready');
    startMediaCachePurgeWorker();
    logger.info('[BOOT] Media cache worker started');
    startPlaybackJobPurgeWorker();
    logger.info('[BOOT] Playback job purge worker started');
  } catch (e) {
    logger.fatal({ err: e }, '[DB] Impossible to connect to database');
    process.exit(1);
  }

  try {
    await fastify.register(require('fastify-socket.io'), {
      cors: {
        allowedHeaders: [
          'Origin',
          'X-Requested-With',
          'Content-Type',
          'Accept',
          'Authorization',
          'forest-context-url',
          'Set-Cookie',
          'set-cookie',
          'Cookie',
        ],
        origin: true,
        credentials: true,
      },
    });

    fastify.addHook('onClose', async (err) => {
      if (err) {
        logger.debug(err);
      }
      await global.prisma.$disconnect();
      await fastify.io.close();
      logger.debug('Server is shutting down');
    });
    logger.info('[BOOT] Socket.IO plugin registered');
  } catch (error) {
    logger.fatal('Impossible to disconnect to db');
  }

  // SERVER CONFIGURATION
  await fastify.register(FastifyCORS, {
    methods: ['GET', 'PUT', 'DELETE', 'POST', 'OPTIONS', 'PATCH'],
    allowedHeaders: [
      'Origin',
      'X-Requested-With',
      'Content-Type',
      'Accept',
      'Authorization',
      'forest-context-url',
      'Set-Cookie',
      'set-cookie',
      'Cookie',
    ],
    origin: true,
    credentials: true,
  });
  logger.info('[BOOT] CORS registered');

  fastify.get('/', async () => {
    return {
      service: 'livechat-bot',
      status: 'ok',
    };
  });

  fastify.get('/health', async () => {
    return {
      status: 'ok',
    };
  });

  fastify.setErrorHandler(async (error, request, reply) => {
    const rawStatusCode = (error as { statusCode?: number })?.statusCode;
    const statusCode =
      typeof rawStatusCode === 'number' && Number.isFinite(rawStatusCode) && rawStatusCode >= 400 && rawStatusCode < 600
        ? Math.trunc(rawStatusCode)
        : 500;

    if (statusCode >= 500) {
      logger.error({ err: error, method: request.method, path: request.url }, '[HTTP] Internal error');
    } else {
      logger.warn(
        { method: request.method, path: request.url, statusCode, message: error.message },
        '[HTTP] Request error',
      );
    }

    if (reply.sent) {
      return;
    }

    const payload = statusCode >= 500 ? { error: 'internal_error' } : { error: 'request_error' };
    return reply.code(statusCode).send(payload);
  });

  loadRosetty();
  logger.info('[BOOT] I18N loaded');
  await loadSocket(fastify);
  logger.info('[BOOT] Socket loader ready');
  await loadRoutes(fastify);
  logger.info('[BOOT] REST routes ready');
  void loadDiscord(fastify).catch((error) => {
    logger.error(error, '[DISCORD] Initialization failed');
  });
  logger.info('[BOOT] Discord bootstrap started');
  gracefulServer.setReady();
  logger.info('[BOOT] Server bootstrap completed');

  return fastify;
};
