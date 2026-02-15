import { timingSafeEqual } from 'crypto';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { toMediaIngestionError } from '../../services/media/mediaErrors';
import { getBearerTokenFromRequest } from '../../services/overlayAuth';
import { createPlaybackJob } from '../../services/playbackJobs';

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

      if (url || media) {
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

      if (!mediaAsset && !text) {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      try {
        const job = await createPlaybackJob({
          guildId,
          mediaAsset,
          text,
          showText: showText ?? !!text,
          authorName: authorName || 'iOS Shortcut',
          authorImage,
          durationSec,
        });

        return reply.code(201).send({
          accepted: true,
          jobId: job.id,
          guildId,
          hasMedia: !!mediaAsset,
          hasText: !!text,
        });
      } catch (error) {
        logger.error(
          {
            err: error,
            guildId,
            hasMedia: !!mediaAsset,
            hasText: !!text,
          },
          '[INGEST] Failed to create playback job',
        );

        return reply.code(500).send({
          error: 'internal_error',
        });
      }
    });
  };
