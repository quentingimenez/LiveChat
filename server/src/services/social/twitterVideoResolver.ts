import fetch from 'node-fetch';
import mime from 'mime-types';
import { normalizeTweetStatusUrl } from './twitterOEmbed';

interface ResolvedTweetVideoMedia {
  url: string;
  mime: string;
  isVertical: boolean;
}

interface SyndicationVariant {
  url?: unknown;
  src?: unknown;
  content_type?: unknown;
  type?: unknown;
  bitrate?: unknown;
}

interface SyndicationMediaDetails {
  original_info?: {
    width?: unknown;
    height?: unknown;
  };
  video_info?: {
    variants?: unknown;
    aspect_ratio?: unknown;
  };
}

interface SyndicationTweetPayload {
  mediaDetails?: unknown;
  video?: {
    aspectRatio?: unknown;
    variants?: unknown;
  };
}

const decodeHtmlAttribute = (value: string) => {
  return value
    .replace(/&amp;/g, '&')
    .replace(/&#38;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&#34;/g, '"')
    .replace(/&#39;/g, "'")
    .trim();
};

const escapeRegex = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const extractMetaContent = (html: string, key: string) => {
  const escapedKey = escapeRegex(key);
  const regex = new RegExp(
    `<meta[^>]*(?:property|name)=["']${escapedKey}["'][^>]*content=["']([^"']+)["'][^>]*>|<meta[^>]*content=["']([^"']+)["'][^>]*(?:property|name)=["']${escapedKey}["'][^>]*>`,
    'i',
  );

  const match = html.match(regex);
  const content = match?.[1] || match?.[2] || '';
  return content ? decodeHtmlAttribute(content) : null;
};

const fetchTextWithTimeout = async (url: string) => {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => {
      controller.abort();
    },
    Math.max(5000, env.MEDIA_DOWNLOAD_TIMEOUT_MS),
  );

  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: {
        'user-agent': 'LiveChatBot/1.0 (+https://github.com/WardenPro/LiveChat-Bot)',
      },
    });

    if (!response.ok) {
      return null;
    }

    return await response.text();
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }
};

const fetchJsonWithTimeout = async <T>(url: string): Promise<T | null> => {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => {
      controller.abort();
    },
    Math.max(5000, env.MEDIA_DOWNLOAD_TIMEOUT_MS),
  );

  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: {
        'user-agent': 'LiveChatBot/1.0 (+https://github.com/WardenPro/LiveChat-Bot)',
      },
    });

    if (!response.ok) {
      return null;
    }

    return (await response.json()) as T;
  } catch {
    return null;
  } finally {
    clearTimeout(timeout);
  }
};

const inferMime = (videoUrl: string) => {
  try {
    const parsed = new URL(videoUrl);
    const pathname = parsed.pathname || '';
    const ext = pathname.includes('.') ? pathname.split('.').pop() : '';
    const guessed = ext ? mime.lookup(ext) : null;

    if (typeof guessed === 'string' && guessed.startsWith('video/')) {
      return guessed;
    }
  } catch {
    // Ignore URL parse errors.
  }

  return 'video/mp4';
};

const toAbsoluteUrl = (rawUrl: string, baseUrl: string) => {
  try {
    return new URL(rawUrl, baseUrl).toString();
  } catch {
    return null;
  }
};

const extractTweetStatusId = (tweetUrl: string) => {
  const match = tweetUrl.match(/\/status\/(\d+)/i);
  return match?.[1] || null;
};

const toNumber = (value: unknown) => {
  const parsed = typeof value === 'number' ? value : typeof value === 'string' ? parseInt(value, 10) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : null;
};

const asNonEmptyString = (value: unknown) => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const resolveLanguage = () => {
  const normalized = (env.I18N || 'en').toLowerCase().trim();
  return normalized.startsWith('fr') ? 'fr' : 'en';
};

const collectSyndicationVariants = (payload: SyndicationTweetPayload) => {
  const variants: Array<{ url: string; contentType: string; bitrate: number }> = [];

  const appendVariant = (variant: SyndicationVariant) => {
    const url = asNonEmptyString(variant.url) || asNonEmptyString(variant.src);
    if (!url) {
      return;
    }

    const contentType = (asNonEmptyString(variant.content_type) || asNonEmptyString(variant.type) || '').toLowerCase();
    const bitrate = toNumber(variant.bitrate) || 0;

    variants.push({
      url,
      contentType,
      bitrate,
    });
  };

  if (Array.isArray(payload.video?.variants)) {
    for (const entry of payload.video.variants as SyndicationVariant[]) {
      appendVariant(entry);
    }
  }

  if (Array.isArray(payload.mediaDetails)) {
    for (const media of payload.mediaDetails as SyndicationMediaDetails[]) {
      if (!Array.isArray(media.video_info?.variants)) {
        continue;
      }

      for (const entry of media.video_info.variants as SyndicationVariant[]) {
        appendVariant(entry);
      }
    }
  }

  return variants;
};

const resolveVerticalFromSyndication = (payload: SyndicationTweetPayload) => {
  if (Array.isArray(payload.mediaDetails)) {
    for (const media of payload.mediaDetails as SyndicationMediaDetails[]) {
      const width = toNumber(media.original_info?.width) || 0;
      const height = toNumber(media.original_info?.height) || 0;

      if (width > 0 && height > 0) {
        return height > width;
      }

      if (Array.isArray(media.video_info?.aspect_ratio) && media.video_info.aspect_ratio.length >= 2) {
        const ratioW = toNumber(media.video_info.aspect_ratio[0]) || 0;
        const ratioH = toNumber(media.video_info.aspect_ratio[1]) || 0;
        if (ratioW > 0 && ratioH > 0) {
          return ratioH > ratioW;
        }
      }
    }
  }

  if (Array.isArray(payload.video?.aspectRatio) && payload.video.aspectRatio.length >= 2) {
    const ratioW = toNumber(payload.video.aspectRatio[0]) || 0;
    const ratioH = toNumber(payload.video.aspectRatio[1]) || 0;

    if (ratioW > 0 && ratioH > 0) {
      return ratioH > ratioW;
    }
  }

  return false;
};

const resolveViaSyndication = async (statusId: string): Promise<ResolvedTweetVideoMedia | null> => {
  const endpoint = new URL('https://cdn.syndication.twimg.com/tweet-result');
  endpoint.searchParams.set('id', statusId);
  endpoint.searchParams.set('lang', resolveLanguage());
  endpoint.searchParams.set('token', '1');

  const payload = await fetchJsonWithTimeout<SyndicationTweetPayload>(endpoint.toString());

  if (!payload || typeof payload !== 'object') {
    return null;
  }

  const variants = collectSyndicationVariants(payload);

  if (variants.length === 0) {
    return null;
  }

  const mp4Variants = variants
    .filter((variant) => variant.contentType.includes('video/mp4'))
    .sort((a, b) => b.bitrate - a.bitrate);

  const picked = mp4Variants[0] || variants.find((variant) => variant.contentType.includes('mpegurl')) || variants[0];

  if (!picked?.url) {
    return null;
  }

  return {
    url: picked.url,
    mime: picked.contentType || inferMime(picked.url),
    isVertical: resolveVerticalFromSyndication(payload),
  };
};

export const resolveTweetVideoMediaFromUrl = async (
  rawUrl?: string | null,
): Promise<ResolvedTweetVideoMedia | null> => {
  if (!rawUrl) {
    return null;
  }

  const normalizedTweetUrl = normalizeTweetStatusUrl(rawUrl);

  if (!normalizedTweetUrl) {
    return null;
  }

  const statusId = extractTweetStatusId(normalizedTweetUrl);
  if (statusId) {
    const syndicationMedia = await resolveViaSyndication(statusId);
    if (syndicationMedia) {
      return syndicationMedia;
    }
  }

  let tweetUrl: URL;
  try {
    tweetUrl = new URL(normalizedTweetUrl);
  } catch {
    return null;
  }

  let resolverUrl: URL;
  try {
    resolverUrl = new URL(tweetUrl.pathname, env.TWITTER_VIDEO_RESOLVER_BASE_URL);
  } catch {
    return null;
  }

  const html = await fetchTextWithTimeout(resolverUrl.toString());

  if (!html) {
    return null;
  }

  const videoCandidate =
    extractMetaContent(html, 'og:video:secure_url') ||
    extractMetaContent(html, 'og:video:url') ||
    extractMetaContent(html, 'og:video') ||
    extractMetaContent(html, 'twitter:player:stream');

  if (!videoCandidate) {
    return null;
  }

  const mediaUrl = toAbsoluteUrl(videoCandidate, resolverUrl.toString());

  if (!mediaUrl) {
    return null;
  }

  const widthValue = extractMetaContent(html, 'og:video:width');
  const heightValue = extractMetaContent(html, 'og:video:height');
  const width = widthValue ? parseInt(widthValue, 10) : 0;
  const height = heightValue ? parseInt(heightValue, 10) : 0;

  return {
    url: mediaUrl,
    mime: inferMime(mediaUrl),
    isVertical: width > 0 && height > width,
  };
};
