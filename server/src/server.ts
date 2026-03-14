/* eslint-disable @typescript-eslint/no-var-requires */
import 'reflect-metadata';
import Fastify from 'fastify';
import FastifyCORS from '@fastify/cors';
import FastifyHelmet from '@fastify/helmet';
import FastifyRateLimit from '@fastify/rate-limit';
import GracefulServer from '@gquittet/graceful-server';
import { Server as SocketIoServer } from 'socket.io';
import { loadRoutes } from './loaders/RESTLoader';
import { loadSocket } from './loaders/socketLoader';
import { loadDiscord } from './loaders/DiscordLoader';
import { loadRosetty } from './services/i18n/loader';
import { loadPrismaClient } from './services/prisma/loadPrisma';
import { ensureMediaStorageDir, startMediaCachePurgeWorker } from './services/media/mediaCache';
import { startPlaybackJobPurgeWorker } from './services/playbackJobs';
import { startPairingCodePurgeWorker } from './services/pairingCodes';
import { startDiscordUserSyncWorker } from './loaders/discordUserSyncLoader';
import { initializePlaybackScheduler } from './services/playbackScheduler';
import { isDevelopmentEnv, getCorsOrigins } from './services/env';

const corsAllowedHeaders = [
  'Origin',
  'X-Requested-With',
  'Content-Type',
  'Accept',
  'Authorization',
  'forest-context-url',
  'Set-Cookie',
  'set-cookie',
  'Cookie',
];

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
    startPairingCodePurgeWorker();
    logger.info('[BOOT] Pairing code purge worker started');
    startDiscordUserSyncWorker();
    logger.info('[BOOT] Discord user sync worker started');
  } catch (e) {
    logger.fatal({ err: e }, '[DB] Impossible to connect to database');
    process.exit(1);
  }

  // Configure CORS origins from environment
  // Automatically includes API_URL origin + any additional origins
  const corsOrigins = getCorsOrigins();
  const isDev = isDevelopmentEnv();

  const getCorsOrigin = (): boolean | string | string[] | RegExp => {
    // In development, allow all origins for easier testing
    if (isDev) {
      logger.warn({
        message: '[SECURITY] Development mode - allowing all CORS origins. Configured origins:',
        origins: corsOrigins.join(', '),
      });
      return true;
    }

    // In production, use strict whitelist
    logger.info({
      message: '[SECURITY] Production mode - CORS restricted to:',
      origins: corsOrigins.join(', '),
    });
    return corsOrigins;
  };

  try {
    const io = new SocketIoServer<ClientToServerEvents, ServerToClientEvents, InterServerEvents, SocketData>(
      fastify.server,
      {
        cors: {
          allowedHeaders: corsAllowedHeaders,
          origin: getCorsOrigin(),
          credentials: true,
        },
      },
    );

    fastify.decorate('io', io);

    fastify.addHook('onClose', async () => {
      await global.prisma.$disconnect();
      await io.close();
      logger.debug('Server is shutting down');
    });
    logger.info('[BOOT] Socket.IO server initialized');
  } catch (error) {
    logger.fatal({ err: error }, 'Impossible to initialize socket server');
  }

  // SERVER CONFIGURATION
  // Security Headers
  await fastify.register(FastifyHelmet, {
    contentSecurityPolicy: {
      directives: {
        defaultSrc: ["'self'"],
        styleSrc: ["'self'", "'unsafe-inline'"],
        scriptSrc: ["'self'", "'unsafe-inline'"],
        imgSrc: ["'self'", 'data:', 'https:'],
        connectSrc: ["'self'"],
        fontSrc: ["'self'"],
        objectSrc: ["'none'"],
        mediaSrc: ["'self'"],
        frameSrc: ["'none'"],
        upgradeInsecureRequests: null, // Allow HTTP for local development
      },
    },
    crossOriginEmbedderPolicy: false, // Required for Socket.IO
    crossOriginResourcePolicy: { policy: 'cross-origin' }, // Required for CORS
  });
  logger.info('[BOOT] Security headers (Helmet) registered');

  // Rate Limiting
  await fastify.register(FastifyRateLimit, {
    max: 100,
    timeWindow: '15 minutes',
    cache: 10000,
    allowList: (req) => {
      // Exempt admin routes from rate limiting (auto-refresh every 2s)
      if (req.url.startsWith('/admin')) {
        return true;
      }
      // Allow local requests in development
      if (isDevelopmentEnv()) {
        const ip = req.ip;
        return ip === '127.0.0.1' || ip === '::1' || ip === '::ffff:127.0.0.1';
      }
      return false;
    },
    skipOnError: false,
  });
  logger.info('[BOOT] Rate limiting registered (100 req/15min, admin routes exempt)');

  // CORS
  await fastify.register(FastifyCORS, {
    methods: ['GET', 'PUT', 'DELETE', 'POST', 'OPTIONS', 'PATCH'],
    allowedHeaders: corsAllowedHeaders,
    origin: getCorsOrigin(),
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
      const message = error instanceof Error ? error.message : String(error);
      logger.warn(
        { method: request.method, path: request.url, statusCode, message },
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
  await initializePlaybackScheduler(fastify);
  logger.info('[BOOT] Playback scheduler initialized');
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
