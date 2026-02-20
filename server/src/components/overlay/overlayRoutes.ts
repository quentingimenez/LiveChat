import fs, { createReadStream } from 'fs';
import type { FastifyReply } from 'fastify';
import { addMinutes } from 'date-fns';
import { createOverlayClientToken, resolveOverlayClientFromRequest } from '../../services/overlayAuth';
import { touchMediaAsset } from '../../services/media/mediaCache';
import { listMemeBoardItems, removeMemeBoardItem } from '../../services/memeBoard';
import { OVERLAY_PROTOCOL_VERSION } from '@livechat/overlay-protocol';

interface ConsumePairingBody {
  code?: unknown;
  deviceName?: unknown;
}

interface MemeBoardItemsQuery {
  q?: unknown;
  limit?: unknown;
  offset?: unknown;
}

const DEFAULT_OVERLAY_DEVICE_PREFIX = 'Overlay';

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized || null;
};

const toOptionalInt = (value: unknown): number | null => {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return Math.floor(value);
  }

  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
};

const streamAssetToReply = async (
  reply: FastifyReply,
  asset: {
    id: string;
    mime: string;
    storagePath: string;
  },
) => {
  if (!fs.existsSync(asset.storagePath)) {
    return reply.code(404).send({
      error: 'media_not_found_on_disk',
    });
  }

  await touchMediaAsset(asset.id);

  reply.header('Cache-Control', 'no-store');
  reply.type(asset.mime);

  return createReadStream(asset.storagePath);
};

export const OverlayRoutes = () =>
  async function (fastify: FastifyCustomInstance) {
    fastify.post<{ Body: ConsumePairingBody }>('/pair/consume', async (request, reply) => {
      const rawCode = toNonEmptyString(request.body?.code)?.toUpperCase() || null;
      const requestedDeviceName = toNonEmptyString(request.body?.deviceName);

      if (!rawCode) {
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

      const authorName = toNonEmptyString((pairingCode as { authorName?: unknown }).authorName);
      const deviceName = requestedDeviceName || `${DEFAULT_OVERLAY_DEVICE_PREFIX}-${authorName || 'User'}`;

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
        deviceName: client.label,
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

      return streamAssetToReply(reply, asset);
    });

    fastify.get<{ Querystring: MemeBoardItemsQuery }>('/meme-board/items', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const list = await listMemeBoardItems({
        guildId: authResult.client.guildId,
        q: toNonEmptyString(request.query?.q),
        limit: toOptionalInt(request.query?.limit),
        offset: toOptionalInt(request.query?.offset),
      });

      return reply.send({
        total: list.total,
        limit: list.limit,
        offset: list.offset,
        items: list.items.map((item) => ({
          id: item.id,
          guildId: item.guildId,
          mediaAssetId: item.mediaAssetId,
          title: item.title,
          createdByName: item.createdByName,
          createdAt: item.createdAt,
          media: {
            id: item.mediaAsset.id,
            kind: item.mediaAsset.kind,
            mime: item.mediaAsset.mime,
            durationSec: item.mediaAsset.durationSec,
            width: item.mediaAsset.width,
            height: item.mediaAsset.height,
            isVertical: item.mediaAsset.isVertical,
            sizeBytes: item.mediaAsset.sizeBytes,
            sourceUrl: item.mediaAsset.sourceUrl,
          },
        })),
      });
    });

    fastify.get<{ Params: { assetId: string } }>('/meme-board/media/:assetId', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const item = await prisma.memeBoardItem.findFirst({
        where: {
          guildId: authResult.client.guildId,
          mediaAssetId: request.params.assetId,
        },
        include: {
          mediaAsset: true,
        },
      });

      if (!item || item.mediaAsset.status !== 'READY' || item.mediaAsset.expiresAt <= new Date()) {
        return reply.code(404).send({
          error: 'media_not_found',
        });
      }

      return streamAssetToReply(reply, item.mediaAsset);
    });

    fastify.delete<{ Params: { itemId: string } }>('/meme-board/items/:itemId', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const result = await removeMemeBoardItem({
        guildId: authResult.client.guildId,
        itemId: request.params.itemId,
      });

      return reply.send({
        deleted: result.deleted,
      });
    });
  };
