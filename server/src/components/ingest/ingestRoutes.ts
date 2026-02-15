import { timingSafeEqual } from 'crypto';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { toMediaIngestionError } from '../../services/media/mediaErrors';
import { getBearerTokenFromRequest } from '../../services/overlayAuth';
import { createPlaybackJob } from '../../services/playbackJobs';
import { encodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { resolveTweetCardFromUrl, resolveTweetCardFromUrlWithOptions } from '../../services/social/twitterOEmbed';
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
  authorName?: unknown;
  authorImage?: unknown;
  durationSec?: unknown;
}

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

const toOptionalDurationSec = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    return null;
  }

  return Math.ceil(value);
};

const isAuthorized = (rawToken: string | null): boolean => {
  const expectedToken = env.INGEST_API_TOKEN.trim();
  const providedToken = rawToken?.trim() || '';

  if (!expectedToken || !providedToken) {
    return false;
  }

  const expectedBuffer = Buffer.from(expectedToken);
  const providedBuffer = Buffer.from(providedToken);

  if (expectedBuffer.length !== providedBuffer.length) {
    return false;
  }

  return timingSafeEqual(expectedBuffer, providedBuffer);
};

export const IngestRoutes = () =>
  async function (fastify: FastifyCustomInstance) {
    fastify.post<{ Body: IngestBody }>('/', async (request, reply) => {
      if (!env.INGEST_API_TOKEN.trim()) {
        return reply.code(503).send({
          error: 'ingest_api_disabled',
        });
      }

      if (!isAuthorized(getBearerTokenFromRequest(request))) {
        return reply.code(401).send({
          error: 'unauthorized',
        });
      }

      const guildId = toNonEmptyString(request.body?.guildId);
      const url = toNonEmptyString(request.body?.url);
      const media = toNonEmptyString(request.body?.media);
      const text = toNonEmptyString(request.body?.text);
      const showText = toOptionalBoolean(request.body?.showText);
      const authorName = toNonEmptyString(request.body?.authorName);
      const authorImage = toNonEmptyString(request.body?.authorImage);
      const durationSec = toOptionalDurationSec(request.body?.durationSec);

      if (!guildId || (!url && !media && !text)) {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      let mediaAsset = null;
      let jobText = text;
      let jobShowText = showText ?? !!text;
      let jobAuthorName = authorName || 'iOS Shortcut';
      let jobAuthorImage = authorImage;
      let jobDurationSec = durationSec;

      if (!media && url) {
        const [tweetVideoResult, tweetCardResult] = await Promise.allSettled([
          resolveTweetVideoMediasFromUrl(url),
          resolveTweetCardFromUrl(url),
        ]);
        const tweetVideoMedias = tweetVideoResult.status === 'fulfilled' ? tweetVideoResult.value : [];
        let tweetCard = tweetCardResult.status === 'fulfilled' ? tweetCardResult.value : null;

        if (tweetVideoResult.status === 'rejected') {
          logger.warn(
            {
              err: toMediaIngestionError(tweetVideoResult.reason),
              sourceUrl: url,
            },
            '[MEDIA] ingest tweet video resolution failed, continuing with fallback',
          );
        }

        if (tweetCardResult.status === 'rejected') {
          logger.warn(
            {
              err: toMediaIngestionError(tweetCardResult.reason),
              sourceUrl: url,
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
          const currentTweetStatusId = extractTweetStatusIdFromUrl(url);
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
              tweetCard = (await resolveTweetCardFromUrlWithOptions(url, { hideMedia: true })) || tweetCard;
            } catch (error) {
              logger.warn(
                {
                  err: toMediaIngestionError(error),
                  sourceUrl: url,
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
            });
          } catch (error) {
            const mediaError = toMediaIngestionError(error);

            logger.error(
              {
                err: mediaError,
                guildId,
                sourceUrl: url,
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
          mediaAsset = await ingestMediaFromSource({ url, media });
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
