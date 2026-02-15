import fs, { createReadStream } from 'fs';
import { addMinutes } from 'date-fns';
import { createOverlayClientToken, resolveOverlayClientFromRequest } from '../../services/overlayAuth';
import { touchMediaAsset } from '../../services/media/mediaCache';
import { OVERLAY_PROTOCOL_VERSION } from '@livechat/overlay-protocol';

interface ConsumePairingBody {
  code: string;
  deviceName: string;
}

export const OverlayRoutes = () =>
  async function (fastify: FastifyCustomInstance) {
    fastify.post<{ Body: ConsumePairingBody }>('/pair/consume', async (request, reply) => {
      const rawCode = request.body?.code?.toUpperCase()?.trim();
      const deviceName = request.body?.deviceName?.trim();

      if (!rawCode || !deviceName) {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      await prisma.pairingCode.deleteMany({
        where: {
          expiresAt: {
            lte: new Date(),
          },
        },
      });

      const pairingCode = await prisma.pairingCode.findFirst({
        where: {
          code: rawCode,
          usedAt: null,
          expiresAt: {
            gt: new Date(),
          },
        },
      });

      if (!pairingCode) {
        return reply.code(404).send({
          error: 'pairing_code_invalid_or_expired',
        });
      }

      await prisma.pairingCode.update({
        where: {
          code: pairingCode.code,
        },
        data: {
          usedAt: new Date(),
          expiresAt: addMinutes(new Date(), -1),
        },
      });

      await prisma.overlayClient.updateMany({
        where: {
          guildId: pairingCode.guildId,
          label: deviceName,
          revokedAt: null,
        },
        data: {
          revokedAt: new Date(),
        },
      });

      const { client, rawToken } = await createOverlayClientToken({
        guildId: pairingCode.guildId,
        label: deviceName,
      });

      return reply.send({
        clientToken: rawToken,
        clientId: client.id,
        guildId: pairingCode.guildId,
        apiBaseUrl: env.API_URL,
      });
    });

    fastify.get('/config', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const guild = await prisma.guild.findFirst({
        where: {
          id: authResult.client.guildId,
        },
      });

      return reply.send({
        guildId: authResult.client.guildId,
        protocolVersion: OVERLAY_PROTOCOL_VERSION,
        showTextDefault: true,
        defaultMediaTime: guild?.defaultMediaTime ?? env.DEFAULT_DURATION,
        maxMediaTime: guild?.maxMediaTime ?? null,
      });
    });

    fastify.get<{ Params: { assetId: string } }>('/media/:assetId', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const asset = await prisma.mediaAsset.findUnique({
        where: {
          id: request.params.assetId,
        },
      });

      if (!asset || asset.status !== 'READY' || asset.expiresAt <= new Date()) {
        return reply.code(404).send({
          error: 'media_not_found',
        });
      }

      if (!fs.existsSync(asset.storagePath)) {
        return reply.code(404).send({
          error: 'media_not_found_on_disk',
        });
      }

      const accessLookupStartedAtMs = Date.now();
      const access = await prisma.playbackJob.findFirst({
        where: {
          mediaAssetId: asset.id,
          guildId: authResult.client.guildId,
        },
        select: {
          id: true,
        },
      });
      const accessLookupDurationMs = Date.now() - accessLookupStartedAtMs;

      if (accessLookupDurationMs >= 250) {
        logger.warn(
          {
            guildId: authResult.client.guildId,
            assetId: asset.id,
            accessLookupDurationMs,
          },
          '[OVERLAY] Slow playback access lookup',
        );
      }

      if (!access) {
        return reply.code(403).send({
          error: 'forbidden',
        });
      }

      await touchMediaAsset(asset.id);

      reply.header('Cache-Control', 'no-store');
      reply.type(asset.mime);

      return createReadStream(asset.storagePath);
    });
  };
