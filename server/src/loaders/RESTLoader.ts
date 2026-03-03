import { startCase } from 'lodash';
import { OverlayRoutes } from '../components/overlay/overlayRoutes';
import { IngestRoutes } from '../components/ingest/ingestRoutes';
import { AdminRoutes } from '../components/admin/adminRoutes';

export const loadRoutes = (fastify: FastifyCustomInstance) => {
  const routes = [
    {
      '/admin': AdminRoutes,
    },
    {
      '/overlay': OverlayRoutes,
    },
    {
      '/ingest': IngestRoutes,
    },
  ];

  for (const route of routes) {
    const [[prefix, fastifyRoutes]] = Object.entries(route);
    fastify.register(fastifyRoutes(), { prefix });
    const routeName = startCase(prefix.substring(1).replaceAll('/', ' '));
    logger.info(`[REST] ${routeName} Routes loaded (${prefix})`);
  }
};
