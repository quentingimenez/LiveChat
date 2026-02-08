const RICH_PAYLOAD_PREFIX = '__LC_OVERLAY_RICH__::';

export interface TweetCardPayload {
  url: string;
  html: string;
  authorName: string;
  authorUrl: string | null;
}

export interface OverlayRichPayload {
  type: 'tweet';
  tweetCard: TweetCardPayload;
  caption?: string | null;
}

const isNonEmptyString = (value: unknown): value is string => {
  return typeof value === 'string' && value.trim().length > 0;
};

const toStringOrNull = (value: unknown): string | null => {
  if (!isNonEmptyString(value)) {
    return null;
  }

  return value.trim();
};

const normalizeRichPayload = (value: unknown): OverlayRichPayload | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const payload = value as {
    type?: unknown;
    caption?: unknown;
    tweetCard?: {
      url?: unknown;
      html?: unknown;
      authorName?: unknown;
      authorUrl?: unknown;
    };
  };

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

  return {
    type: 'tweet',
    tweetCard: {
      url: payload.tweetCard.url.trim(),
      html: payload.tweetCard.html.trim(),
      authorName,
      authorUrl: toStringOrNull(payload.tweetCard.authorUrl),
    },
    caption: toStringOrNull(payload.caption),
  };
};

export const encodeRichOverlayPayload = (payload: OverlayRichPayload): string => {
  const rawPayload = JSON.stringify(payload);
  const encoded = Buffer.from(rawPayload, 'utf8').toString('base64');
  return `${RICH_PAYLOAD_PREFIX}${encoded}`;
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

