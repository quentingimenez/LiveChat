const RICH_PAYLOAD_PREFIX = '__LC_OVERLAY_RICH__::';

export interface TweetCardPayload {
  url: string;
  html: string;
  authorName: string;
  authorUrl: string | null;
  currentStatusId?: string | null;
  videoUrl?: string | null;
  videoMime?: string | null;
  videoIsVertical?: boolean | null;
  videos?: Array<{
    url: string;
    mime: string | null;
    isVertical: boolean | null;
    sourceStatusId: string | null;
    durationSec: number | null;
  }>;
}

export interface TweetOverlayRichPayload {
  type: 'tweet';
  tweetCard: TweetCardPayload;
  caption?: string | null;
}

export interface MediaOverlayRichPayload {
  type: 'media';
  caption?: string | null;
  startOffsetSec?: number | null;
}

export type OverlayRichPayload = TweetOverlayRichPayload | MediaOverlayRichPayload;

const isNonEmptyString = (value: unknown): value is string => {
  return typeof value === 'string' && value.trim().length > 0;
};

const toStringOrNull = (value: unknown): string | null => {
  if (!isNonEmptyString(value)) {
    return null;
  }

  return value.trim();
};

const toOptionalPositiveInt = (value: unknown): number | null => {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return null;
  }

  const normalized = Math.floor(value);
  return normalized > 0 ? normalized : null;
};

const normalizeRichPayload = (value: unknown): OverlayRichPayload | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const payload = value as {
    type?: unknown;
    caption?: unknown;
    startOffsetSec?: unknown;
    tweetCard?: {
      url?: unknown;
      html?: unknown;
      authorName?: unknown;
      authorUrl?: unknown;
      currentStatusId?: unknown;
      videoUrl?: unknown;
      videoMime?: unknown;
      videoIsVertical?: unknown;
      videos?: unknown;
    };
  };

  if (payload.type === 'media') {
    const caption = toStringOrNull(payload.caption);
    const startOffsetSec = toOptionalPositiveInt(payload.startOffsetSec);

    if (!caption && startOffsetSec === null) {
      return null;
    }

    return {
      type: 'media',
      caption,
      startOffsetSec,
    };
  }

  if (payload.type !== 'tweet') {
    return null;
  }

  if (!payload.tweetCard) {
    return null;
  }

  if (!isNonEmptyString(payload.tweetCard.url) || !isNonEmptyString(payload.tweetCard.html)) {
    return null;
  }

  const authorName = isNonEmptyString(payload.tweetCard.authorName) ? payload.tweetCard.authorName.trim() : 'Tweet';
  const videos = Array.isArray(payload.tweetCard.videos)
    ? payload.tweetCard.videos
        .map((video) => {
          if (!video || typeof video !== 'object') {
            return null;
          }

          const candidate = video as {
            url?: unknown;
            mime?: unknown;
            isVertical?: unknown;
            sourceStatusId?: unknown;
            durationSec?: unknown;
          };

          if (!isNonEmptyString(candidate.url)) {
            return null;
          }

          const rawDuration =
            typeof candidate.durationSec === 'number' && Number.isFinite(candidate.durationSec)
              ? candidate.durationSec
              : null;

          return {
            url: candidate.url.trim(),
            mime: toStringOrNull(candidate.mime),
            isVertical: typeof candidate.isVertical === 'boolean' ? candidate.isVertical : null,
            sourceStatusId: toStringOrNull(candidate.sourceStatusId),
            durationSec: rawDuration && rawDuration > 0 ? rawDuration : null,
          };
        })
        .filter(
          (
            video,
          ): video is {
            url: string;
            mime: string | null;
            isVertical: boolean | null;
            sourceStatusId: string | null;
            durationSec: number | null;
          } => !!video,
        )
    : [];

  return {
    type: 'tweet',
    tweetCard: {
      url: payload.tweetCard.url.trim(),
      html: payload.tweetCard.html.trim(),
      authorName,
      authorUrl: toStringOrNull(payload.tweetCard.authorUrl),
      currentStatusId: toStringOrNull(payload.tweetCard.currentStatusId),
      videoUrl: toStringOrNull(payload.tweetCard.videoUrl),
      videoMime: toStringOrNull(payload.tweetCard.videoMime),
      videoIsVertical:
        typeof payload.tweetCard.videoIsVertical === 'boolean' ? payload.tweetCard.videoIsVertical : null,
      videos,
    },
    caption: toStringOrNull(payload.caption),
  };
};

export const encodeRichOverlayPayload = (payload: OverlayRichPayload): string => {
  const rawPayload = JSON.stringify(payload);
  const encoded = Buffer.from(rawPayload, 'utf8').toString('base64');
  return `${RICH_PAYLOAD_PREFIX}${encoded}`;
};

export const buildMediaOverlayTextPayload = (params: {
  text?: string | null;
  startOffsetSec?: number | null;
}): string | null => {
  const rawText = typeof params.text === 'string' ? params.text : null;
  const startOffsetSec = toOptionalPositiveInt(params.startOffsetSec);

  if (startOffsetSec === null) {
    return rawText;
  }

  const caption = toStringOrNull(rawText);

  return encodeRichOverlayPayload({
    type: 'media',
    caption,
    startOffsetSec,
  });
};

export const decodeRichOverlayPayload = (value?: string | null): OverlayRichPayload | null => {
  if (!isNonEmptyString(value) || !value.startsWith(RICH_PAYLOAD_PREFIX)) {
    return null;
  }

  const encoded = value.slice(RICH_PAYLOAD_PREFIX.length);

  if (!isNonEmptyString(encoded)) {
    return null;
  }

  try {
    const json = Buffer.from(encoded, 'base64').toString('utf8');
    const parsed = JSON.parse(json);
    return normalizeRichPayload(parsed);
  } catch {
    return null;
  }
};
