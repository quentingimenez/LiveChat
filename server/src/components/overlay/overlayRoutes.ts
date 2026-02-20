import fs, { createReadStream } from 'fs';
import type { FastifyReply, FastifyRequest } from 'fastify';
import { addMinutes } from 'date-fns';
import { OVERLAY_PROTOCOL_VERSION } from '@livechat/overlay-protocol';
import { createOverlayClientToken, resolveOverlayClientFromRequest } from '../../services/overlayAuth';
import { touchMediaAsset } from '../../services/media/mediaCache';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { toMediaIngestionError } from '../../services/media/mediaErrors';
import {
  addToMemeBoard,
  listMemeBoardItems,
  removeMemeBoardItem,
  updateMemeBoardItemTitle,
} from '../../services/memeBoard';

interface ConsumePairingBody {
  code?: unknown;
  deviceName?: unknown;
}

interface MemeBoardItemsQuery {
  q?: unknown;
  limit?: unknown;
  offset?: unknown;
}

interface MemeBoardItemCreateBody {
  url?: unknown;
  title?: unknown;
  forceRefresh?: unknown;
}

interface MemeBoardItemUpdateBody {
  title?: unknown;
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

const toBooleanFlag = (value: unknown): boolean => {
  if (typeof value === 'boolean') {
    return value;
  }

  if (typeof value === 'number') {
    return value === 1;
  }

  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
  }

  return false;
};

type MemeBoardListItem = Awaited<ReturnType<typeof listMemeBoardItems>>['items'][number];

const toOverlayMemeBoardItemPayload = (item: MemeBoardListItem) => {
  return {
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
  };
};

const parseByteRange = (
  rawRangeHeader: string,
  totalSize: number,
): { start: number; end: number } | 'invalid' | null => {
  const trimmed = rawRangeHeader.trim();

  if (!trimmed) {
    return null;
  }

  if (!trimmed.toLowerCase().startsWith('bytes=')) {
    return 'invalid';
  }

  const firstRange = trimmed.slice(6).split(',')[0]?.trim() || '';
  if (!firstRange) {
    return 'invalid';
  }

  const [rawStart, rawEnd] = firstRange.split('-').map((part) => part.trim());

  if (!rawStart && !rawEnd) {
    return 'invalid';
  }

  if (!rawStart) {
    const suffixLength = Number.parseInt(rawEnd || '', 10);

    if (!Number.isFinite(suffixLength) || suffixLength <= 0) {
      return 'invalid';
    }

    const start = suffixLength >= totalSize ? 0 : totalSize - suffixLength;
    const end = totalSize - 1;
    return {
      start,
      end,
    };
  }

  const start = Number.parseInt(rawStart, 10);

  if (!Number.isFinite(start) || start < 0 || start >= totalSize) {
    return 'invalid';
  }

  if (!rawEnd) {
    return {
      start,
      end: totalSize - 1,
    };
  }

  const parsedEnd = Number.parseInt(rawEnd, 10);

  if (!Number.isFinite(parsedEnd) || parsedEnd < start) {
    return 'invalid';
  }

  return {
    start,
    end: Math.min(parsedEnd, totalSize - 1),
  };
};

const streamAssetToReply = async (
  request: FastifyRequest,
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

  const fileStat = await fs.promises.stat(asset.storagePath).catch(() => null);

  if (!fileStat || !fileStat.isFile()) {
    return reply.code(404).send({
      error: 'media_not_found_on_disk',
    });
  }

  await touchMediaAsset(asset.id);

  const totalSize = fileStat.size;
  const rawRangeHeader = typeof request.headers.range === 'string' ? request.headers.range : '';
  const byteRange = parseByteRange(rawRangeHeader, totalSize);

  reply.header('Cache-Control', 'no-store');
  reply.header('Accept-Ranges', 'bytes');
  reply.type(asset.mime);

  if (byteRange === 'invalid') {
    reply.header('Content-Range', `bytes */${totalSize}`);
    return reply.code(416).send();
  }

  if (byteRange) {
    const chunkSize = byteRange.end - byteRange.start + 1;
    reply.code(206);
    reply.header('Content-Range', `bytes ${byteRange.start}-${byteRange.end}/${totalSize}`);
    reply.header('Content-Length', `${chunkSize}`);
    return createReadStream(asset.storagePath, {
      start: byteRange.start,
      end: byteRange.end,
    });
  }

  reply.header('Content-Length', `${totalSize}`);
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
      const authorImage = toNonEmptyString((pairingCode as { authorImage?: unknown }).authorImage);
      const createdByDiscordUserId = toNonEmptyString(
        (pairingCode as { createdByDiscordUserId?: unknown }).createdByDiscordUserId,
      );
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
        defaultAuthorName: authorName,
        defaultAuthorImage: authorImage,
        createdByDiscordUserId,
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

      return streamAssetToReply(request, reply, asset);
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
        items: list.items.map(toOverlayMemeBoardItemPayload),
      });
    });

    fastify.post<{ Body: MemeBoardItemCreateBody }>('/meme-board/items', async (request, reply) => {
      const authResult = await resolveOverlayClientFromRequest(request);

      if (!authResult) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const url = toNonEmptyString(request.body?.url);
      const title = toNonEmptyString(request.body?.title);
      const forceRefresh = toBooleanFlag(request.body?.forceRefresh);

      if (!url) {
        return reply.code(400).send({
          error: 'invalid_payload',
          message: 'url is required',
        });
      }

      let mediaAsset: {
        id: string;
      };

      try {
        mediaAsset = await ingestMediaFromSource({
          url,
          forceRefresh,
        });
      } catch (error) {
        const mediaError = toMediaIngestionError(error);
        return reply.code(422).send({
          error: 'media_ingestion_failed',
          code: mediaError.code,
          message: mediaError.message,
        });
      }

      const result = await addToMemeBoard({
        guildId: authResult.client.guildId,
        mediaAssetId: mediaAsset.id,
        title,
        createdByDiscordUserId: toNonEmptyString(
          (authResult.client as { createdByDiscordUserId?: unknown }).createdByDiscordUserId,
        ),
        createdByName:
          toNonEmptyString((authResult.client as { defaultAuthorName?: unknown }).defaultAuthorName) ||
          authResult.client.label,
      });

      if (!result.item) {
        return reply.code(500).send({
          error: 'meme_board_item_not_found_after_upsert',
        });
      }

      return reply.send({
        created: result.created,
        item: toOverlayMemeBoardItemPayload(result.item as MemeBoardListItem),
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

      return streamAssetToReply(request, reply, item.mediaAsset);
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

    fastify.patch<{ Params: { itemId: string }; Body: MemeBoardItemUpdateBody }>(
      '/meme-board/items/:itemId',
      async (request, reply) => {
        const authResult = await resolveOverlayClientFromRequest(request);

        if (!authResult) {
          return reply.code(401).send({
            error: 'unauthorized',
          });
        }

        const updated = await updateMemeBoardItemTitle({
          guildId: authResult.client.guildId,
          itemId: request.params.itemId,
          title: toNonEmptyString(request.body?.title),
        });

        if (!updated) {
          return reply.code(404).send({
            error: 'item_not_found',
          });
        }

        return reply.send({
          updated: true,
          item: {
            id: updated.id,
            title: updated.title,
          },
        });
      },
    );
  };
