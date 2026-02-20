import { addMinutes } from 'date-fns';
import {
  createIngestClientToken,
  isIngestApiEnabled,
  resolveIngestAuthFromRequest,
  revokeIngestClientsForGuildLabel,
} from '../../services/ingestAuth';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';
import { encodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { addToMemeBoard } from '../../services/memeBoard';
import {
  normalizeTweetStatusUrl,
  resolveTweetCardFromUrl,
  resolveTweetCardFromUrlWithOptions,
} from '../../services/social/twitterOEmbed';
import {
  extractTweetStatusIdFromUrl,
  resolveTweetVideoMediasFromUrl,
} from '../../services/social/twitterVideoResolver';

interface IngestBody {
  guildId?: unknown;
  url?: unknown;
  media?: unknown;
  text?: unknown;
  showText?: unknown;
  forceRefresh?: unknown;
  authorName?: unknown;
  authorImage?: unknown;
  durationSec?: unknown;
  saveToBoard?: unknown;
}

interface ConsumeIngestPairingBody {
  code?: unknown;
  deviceName?: unknown;
}

const DEFAULT_INGEST_AUTHOR_NAME = 'LiveChat Extension';
const DEFAULT_INGEST_DEVICE_PREFIX = 'Extension';

const getTweetCardDurationSec = (
  medias: Array<{
    durationSec: number | null;
  }>,
) => {
  const knownDurations = medias
    .map((media) =>
      typeof media.durationSec === 'number' && Number.isFinite(media.durationSec) ? media.durationSec : 0,
    )
    .filter((candidateDurationSec) => candidateDurationSec > 0);

  if (knownDurations.length === 0) {
    return 15;
  }

  return Math.max(1, Math.ceil(Math.max(...knownDurations)));
};

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const trimmed = value.trim();

  return trimmed || null;
};

const toOptionalBoolean = (value: unknown): boolean | undefined => {
  if (typeof value === 'boolean') {
    return value;
  }

  return undefined;
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

const toOptionalDurationSec = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    return null;
  }

  return Math.ceil(value);
};

export const IngestRoutes = () =>
  async function (fastify: FastifyCustomInstance) {
    fastify.post<{ Body: ConsumeIngestPairingBody }>('/pair/consume', async (request, reply) => {
      const rawCode = toNonEmptyString(request.body?.code)?.toUpperCase() || null;
      const requestedDeviceName = toNonEmptyString(request.body?.deviceName);
      const codePrefix = rawCode ? rawCode.slice(0, 3) : null;

      logger.info(
        {
          ip: request.ip,
          hasCode: !!rawCode,
          codePrefix,
          requestedDeviceName,
        },
        '[INGEST] Pairing consume request received',
      );

      if (!rawCode) {
        logger.warn(
          {
            ip: request.ip,
            requestedDeviceName,
          },
          '[INGEST] Pairing consume rejected: invalid payload',
        );

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
        logger.warn(
          {
            ip: request.ip,
            codePrefix,
            requestedDeviceName,
          },
          '[INGEST] Pairing consume rejected: code invalid or expired',
        );

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

      const authorName =
        toNonEmptyString((pairingCode as { authorName?: unknown }).authorName) || DEFAULT_INGEST_AUTHOR_NAME;
      const authorImage = toNonEmptyString((pairingCode as { authorImage?: unknown }).authorImage);
      const deviceName = requestedDeviceName || `${DEFAULT_INGEST_DEVICE_PREFIX}-${authorName}`;

      await revokeIngestClientsForGuildLabel(pairingCode.guildId, deviceName);

      const { client, rawToken } = await createIngestClientToken({
        guildId: pairingCode.guildId,
        label: deviceName,
        defaultAuthorName: authorName,
        defaultAuthorImage: authorImage,
        createdByDiscordUserId: pairingCode.createdByDiscordUserId,
      });

      logger.info(
        {
          ip: request.ip,
          codePrefix,
          guildId: pairingCode.guildId,
          deviceName,
          ingestClientId: client.id,
        },
        '[INGEST] Pairing consume succeeded',
      );

      return reply.send({
        apiBaseUrl: env.API_URL,
        ingestApiToken: rawToken,
        ingestClientId: client.id,
        guildId: client.guildId,
        deviceName: client.label,
        authorName: client.defaultAuthorName,
        authorImage: client.defaultAuthorImage,
      });
    });

    fastify.post<{ Body: IngestBody }>('/', async (request, reply) => {
      const authResult = await resolveIngestAuthFromRequest(request);
      if (!authResult) {
        const enabled = await isIngestApiEnabled();

        if (!enabled) {
          return reply.code(503).send({
            error: 'ingest_api_disabled',
          });
        }

        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const requestedGuildId = toNonEmptyString(request.body?.guildId);
      const url = toNonEmptyString(request.body?.url);
      const media = toNonEmptyString(request.body?.media);
      const text = toNonEmptyString(request.body?.text);
      const showText = toOptionalBoolean(request.body?.showText);
      const forceRefresh = toBooleanFlag(request.body?.forceRefresh);
      const saveToBoard = toBooleanFlag(request.body?.saveToBoard);
      const authorName = toNonEmptyString(request.body?.authorName);
      const authorImage = toNonEmptyString(request.body?.authorImage);
      const durationSec = toOptionalDurationSec(request.body?.durationSec);

      if (authResult.kind === 'client' && requestedGuildId && requestedGuildId !== authResult.client.guildId) {
        return reply.code(403).send({
          error: 'guild_mismatch',
        });
      }

      const guildId = authResult.kind === 'client' ? authResult.client.guildId : requestedGuildId;

      if (!guildId || (!url && !media && !text)) {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      if (saveToBoard && !url && !media) {
        return reply.code(400).send({
          error: 'invalid_payload',
          message: 'saveToBoard requires a resolvable media source',
        });
      }

      let mediaAsset: any = null;
      let jobText = text;
      let jobShowText = showText ?? !!text;
      let jobAuthorName: string | null =
        authorName ||
        (authResult.kind === 'client'
          ? toNonEmptyString(authResult.client.defaultAuthorName) || DEFAULT_INGEST_AUTHOR_NAME
          : 'iOS Shortcut');
      let jobAuthorImage =
        authorImage || (authResult.kind === 'client' ? toNonEmptyString(authResult.client.defaultAuthorImage) : null);
      let jobDurationSec = durationSec;
      const normalizedTweetUrl = !saveToBoard && !media && url ? normalizeTweetStatusUrl(url) : null;

      if (normalizedTweetUrl) {
        const [tweetVideoResult, tweetCardResult] = await Promise.allSettled([
          resolveTweetVideoMediasFromUrl(normalizedTweetUrl),
          resolveTweetCardFromUrl(normalizedTweetUrl),
        ]);
        const tweetVideoMedias = tweetVideoResult.status === 'fulfilled' ? tweetVideoResult.value : [];
        let tweetCard = tweetCardResult.status === 'fulfilled' ? tweetCardResult.value : null;

        if (tweetVideoResult.status === 'rejected') {
          logger.warn(
            {
              err: toMediaIngestionError(tweetVideoResult.reason),
              sourceUrl: normalizedTweetUrl,
            },
            '[MEDIA] ingest tweet video resolution failed, continuing with fallback',
          );
        }

        if (tweetCardResult.status === 'rejected') {
          logger.warn(
            {
              err: toMediaIngestionError(tweetCardResult.reason),
              sourceUrl: normalizedTweetUrl,
            },
            '[MEDIA] ingest tweet card resolution failed, continuing with fallback',
          );
        }

        if (tweetCard) {
          const tweetVideosForOverlay = tweetVideoMedias.slice(0, 2).map((video) => ({
            url: video.url,
            mime: video.mime,
            isVertical: video.isVertical,
            sourceStatusId: video.sourceStatusId,
            durationSec: video.durationSec,
          }));
          const currentTweetStatusId = extractTweetStatusIdFromUrl(normalizedTweetUrl);
          const hasCurrentTweetVideo =
            !!currentTweetStatusId &&
            tweetVideosForOverlay.some((video) => video.sourceStatusId === currentTweetStatusId);
          const hasOtherTweetVideo =
            !!currentTweetStatusId &&
            tweetVideosForOverlay.some(
              (video) => !!video.sourceStatusId && video.sourceStatusId !== currentTweetStatusId,
            );
          const shouldHideCardMedia = !!currentTweetStatusId && hasCurrentTweetVideo && hasOtherTweetVideo;

          if (shouldHideCardMedia) {
            try {
              tweetCard =
                (await resolveTweetCardFromUrlWithOptions(normalizedTweetUrl, {
                  hideMedia: true,
                })) || tweetCard;
            } catch (error) {
              logger.warn(
                {
                  err: toMediaIngestionError(error),
                  sourceUrl: normalizedTweetUrl,
                },
                '[MEDIA] ingest tweet card hide-media resolution failed, keeping original card',
              );
            }
          }

          jobText = encodeRichOverlayPayload({
            type: 'tweet',
            tweetCard: {
              ...tweetCard,
              currentStatusId: currentTweetStatusId,
              videoUrl: tweetVideosForOverlay[0]?.url || null,
              videoMime: tweetVideosForOverlay[0]?.mime || null,
              videoIsVertical: tweetVideosForOverlay[0]?.isVertical ?? null,
              videos: tweetVideosForOverlay,
            },
            caption: text || null,
          });
          jobShowText = false;
          jobAuthorName = null;
          jobAuthorImage = null;
          jobDurationSec = durationSec ?? getTweetCardDurationSec(tweetVideoMedias);
        } else if (tweetVideoMedias[0]) {
          try {
            mediaAsset = await ingestMediaFromSource({
              media: tweetVideoMedias[0].url,
              forceRefresh,
            });
          } catch (error) {
            const mediaError = toMediaIngestionError(error);

            logger.error(
              {
                err: mediaError,
                guildId,
                sourceUrl: normalizedTweetUrl,
                sourceMedia: tweetVideoMedias[0].url,
              },
              `[MEDIA] ingest API failed (${mediaError.code})`,
            );

            return reply.code(422).send({
              error: 'media_ingestion_failed',
              code: mediaError.code,
              message: mediaError.message,
            });
          }
        }
      }

      if (!jobText && !mediaAsset && (url || media)) {
        try {
          mediaAsset = await ingestMediaFromSource({ url, media, forceRefresh });
        } catch (error) {
          const mediaError = toMediaIngestionError(error);

          logger.error(
            {
              err: mediaError,
              guildId,
              sourceUrl: url || null,
              sourceMedia: media || null,
            },
            `[MEDIA] ingest API failed (${mediaError.code})`,
          );

          return reply.code(422).send({
            error: 'media_ingestion_failed',
            code: mediaError.code,
            message: mediaError.message,
          });
        }
      }

      if (!mediaAsset && !jobText) {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      if (saveToBoard) {
        if (!mediaAsset) {
          return reply.code(400).send({
            error: 'invalid_payload',
            message: 'saveToBoard requires media',
          });
        }

        try {
          const result = await addToMemeBoard({
            guildId,
            mediaAssetId: mediaAsset.id,
            title: text,
            createdByDiscordUserId:
              authResult.kind === 'client' ? toNonEmptyString(authResult.client.createdByDiscordUserId) : null,
            createdByName: jobAuthorName,
          });

          logger.info(
            {
              guildId,
              mediaAssetId: mediaAsset.id,
              memeBoardItemId: result.item?.id || null,
              created: result.created,
              forceRefresh,
            },
            '[INGEST] Meme board item saved',
          );

          return reply.code(201).send({
            accepted: true,
            savedToBoard: true,
            created: result.created,
            guildId,
            memeBoardItemId: result.item?.id || null,
            mediaAssetId: mediaAsset.id,
          });
        } catch (error) {
          logger.error(
            {
              err: error,
              guildId,
              mediaAssetId: mediaAsset.id,
            },
            '[INGEST] Failed to save meme board item',
          );

          return reply.code(500).send({
            error: 'internal_error',
          });
        }
      }

      try {
        const job = await createPlaybackJob({
          guildId,
          mediaAsset,
          text: jobText,
          showText: jobShowText,
          authorName: jobAuthorName,
          authorImage: jobAuthorImage,
          durationSec: jobDurationSec,
          source: 'ingest_api',
        });

        logger.info(
          {
            guildId,
            jobId: job.id,
            hasMedia: !!mediaAsset,
            hasText: !!jobText,
            showText: jobShowText,
            requestedDurationSec: durationSec ?? null,
            resolvedDurationSec: job.durationSec,
            forceRefresh,
          },
          '[INGEST] Job accepted',
        );

        return reply.code(201).send({
          accepted: true,
          jobId: job.id,
          guildId,
          hasMedia: !!mediaAsset,
          hasText: !!jobText,
        });
      } catch (error) {
        logger.error(
          {
            err: error,
            guildId,
            hasMedia: !!mediaAsset,
            hasText: !!jobText,
          },
          '[INGEST] Failed to create playback job',
        );

        return reply.code(500).send({
          error: 'internal_error',
        });
      }
    });
  };
