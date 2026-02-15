import { runServer } from './server';
import { env } from './services/env';

const logBootError = (message: string, error: unknown) => {
  if (global.logger) {
    logger.fatal({ err: error }, message);
    return;
  }

  // eslint-disable-next-line no-console
  console.error(message, error);
};

process.on('unhandledRejection', (reason) => {
  logBootError('[BOOT] Unhandled rejection', reason);
});

process.on('uncaughtException', (error) => {
  logBootError('[BOOT] Uncaught exception', error);
  process.exit(1);
});

const bootstrap = async () => {
  global.env = env;
  //@ts-ignore
  process.env = env;

  const port: number = env.PORT ? env.PORT : 3000;

  const fastify = await runServer();

  await fastify.listen({
    port,
    host: '::',
    listenTextResolver: (address) => `[SERVER] Listening on ${address}`,
  });
};

void bootstrap().catch((error) => {
  logBootError('[BOOT] Startup failed', error);
  process.exit(1);
});
