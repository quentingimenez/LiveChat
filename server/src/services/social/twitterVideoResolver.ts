import fetch from 'node-fetch';
import mime from 'mime-types';
import { normalizeTweetStatusUrl } from './twitterOEmbed';

interface ResolvedTweetVideoMedia {
  url: string;
  mime: string;
  isVertical: boolean;
  sourceStatusId: string | null;
}

interface ResolveViaSyndicationOptions {
  depth: number;
  visited: Set<string>;
}

interface SyndicationVariant {
  url?: unknown;
  src?: unknown;
  content_type?: unknown;
  type?: unknown;
  bitrate?: unknown;
}

interface SyndicationMediaDetails {
  expanded_url?: unknown;
  url?: unknown;
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

export const extractTweetStatusIdFromUrl = (rawUrl?: string | null) => {
  if (!rawUrl) {
    return null;
  }

  const normalizedTweetUrl = normalizeTweetStatusUrl(rawUrl);
  if (!normalizedTweetUrl) {
    return null;
  }

  return extractTweetStatusId(normalizedTweetUrl);
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
  const variants: Array<{ url: string; contentType: string; bitrate: number; sourceStatusId: string | null }> = [];

  const appendVariant = (variant: SyndicationVariant, sourceStatusId: string | null) => {
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
      sourceStatusId,
    });
  };

  if (Array.isArray(payload.mediaDetails)) {
    const variantsFromCurrentStatus: Array<{
      url: string;
      contentType: string;
      bitrate: number;
      sourceStatusId: string | null;
    }> = [];
    const variantsFromAllStatuses: Array<{
      url: string;
      contentType: string;
      bitrate: number;
      sourceStatusId: string | null;
    }> = [];

    const pushVariant = (
      target: typeof variantsFromCurrentStatus,
      variant: SyndicationVariant,
      sourceStatusId: string | null,
    ) => {
      const url = asNonEmptyString(variant.url) || asNonEmptyString(variant.src);
      if (!url) {
        return;
      }

      const contentType = (
        asNonEmptyString(variant.content_type) ||
        asNonEmptyString(variant.type) ||
        ''
      ).toLowerCase();
      const bitrate = toNumber(variant.bitrate) || 0;
      target.push({
        url,
        contentType,
        bitrate,
        sourceStatusId,
      });
    };

    for (const media of payload.mediaDetails as SyndicationMediaDetails[]) {
      const expandedUrl = asNonEmptyString(media.expanded_url) || asNonEmptyString(media.url);
      const mediaStatusId = expandedUrl ? extractTweetStatusId(expandedUrl) : null;

      if (!Array.isArray(media.video_info?.variants)) {
        continue;
      }

      for (const entry of media.video_info.variants as SyndicationVariant[]) {
        pushVariant(variantsFromAllStatuses, entry, mediaStatusId);
      }
    }

    const statusIdValue = `${(payload as Record<string, unknown>).id_str || ''}`.trim();
    const hasCurrentStatusId = /^\d+$/.test(statusIdValue);

    if (hasCurrentStatusId) {
      for (const variant of variantsFromAllStatuses) {
        if (variant.sourceStatusId === statusIdValue) {
          variantsFromCurrentStatus.push(variant);
        }
      }
    }

    if (variantsFromCurrentStatus.length > 0) {
      variants.push(...variantsFromCurrentStatus);
    } else {
      variants.push(...variantsFromAllStatuses);
    }
  }

  if (variants.length === 0 && Array.isArray(payload.video?.variants)) {
    for (const entry of payload.video.variants as SyndicationVariant[]) {
      appendVariant(entry, null);
    }
  }

  return variants;
};

const collectVariantsRecursively = (value: unknown, maxDepth = 6) => {
  const variants: Array<{ url: string; contentType: string; bitrate: number; sourceStatusId: string | null }> = [];
  const visited = new Set<unknown>();

  const walk = (node: unknown, depth: number) => {
    if (depth > maxDepth || node === null || node === undefined || visited.has(node)) {
      return;
    }

    if (typeof node !== 'object') {
      return;
    }

    visited.add(node);

    if (Array.isArray(node)) {
      for (const entry of node) {
        walk(entry, depth + 1);
      }
      return;
    }

    const record = node as Record<string, unknown>;

    if (Array.isArray(record.variants)) {
      for (const entry of record.variants as SyndicationVariant[]) {
        const url = asNonEmptyString(entry.url) || asNonEmptyString(entry.src);
        if (!url) {
          continue;
        }

        const contentType = (asNonEmptyString(entry.content_type) || asNonEmptyString(entry.type) || '').toLowerCase();
        const bitrate = toNumber(entry.bitrate) || 0;
        variants.push({
          url,
          contentType,
          bitrate,
          sourceStatusId: null,
        });
      }
    }

    for (const child of Object.values(record)) {
      walk(child, depth + 1);
    }
  };

  walk(value, 0);
  return variants;
};

const dedupeVariants = (
  variants: Array<{ url: string; contentType: string; bitrate: number; sourceStatusId: string | null }>,
) => {
  const deduped = new Map<
    string,
    { url: string; contentType: string; bitrate: number; sourceStatusId: string | null }
  >();

  for (const variant of variants) {
    const key = variant.url.trim();
    const existing = deduped.get(key);

    if (
      !existing ||
      variant.bitrate > existing.bitrate ||
      (variant.bitrate === existing.bitrate && variant.sourceStatusId && !existing.sourceStatusId)
    ) {
      deduped.set(key, variant);
    }
  }

  return Array.from(deduped.values());
};

const collectReferencedStatusIds = (payload: Record<string, unknown>) => {
  const refs: string[] = [];
  const add = (candidate: unknown) => {
    const value = `${candidate || ''}`.trim();
    if (!/^\d+$/.test(value)) {
      return;
    }
    if (!refs.includes(value)) {
      refs.push(value);
    }
  };

  add(payload.in_reply_to_status_id_str);
  add(payload.in_reply_to_status_id);
  add(payload.quoted_status_id_str);
  add(payload.quoted_status_id);

  const referencedTweets = payload.referenced_tweets;
  if (Array.isArray(referencedTweets)) {
    for (const entry of referencedTweets) {
      if (entry && typeof entry === 'object') {
        add((entry as Record<string, unknown>).id_str);
        add((entry as Record<string, unknown>).id);
      }
    }
  }

  const quotedTweet = payload.quoted_tweet;
  if (quotedTweet && typeof quotedTweet === 'object') {
    const quotedRecord = quotedTweet as Record<string, unknown>;
    add(quotedRecord.id_str);
    add(quotedRecord.id);
  }

  const collectFromStrings = (node: unknown, depth = 0) => {
    if (depth > 5) {
      return;
    }

    if (typeof node === 'string') {
      const match = node.match(/\/status\/(\d+)/i);
      if (match?.[1]) {
        add(match[1]);
      }
      return;
    }

    if (!node || typeof node !== 'object') {
      return;
    }

    if (Array.isArray(node)) {
      for (const entry of node) {
        collectFromStrings(entry, depth + 1);
      }
      return;
    }

    for (const value of Object.values(node as Record<string, unknown>)) {
      collectFromStrings(value, depth + 1);
    }
  };

  collectFromStrings(payload);

  return refs;
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

const resolveViaSyndication = async (
  statusId: string,
  options: ResolveViaSyndicationOptions = {
    depth: 0,
    visited: new Set<string>(),
  },
): Promise<ResolvedTweetVideoMedia | null> => {
  if (options.depth > 2 || options.visited.has(statusId)) {
    return null;
  }

  options.visited.add(statusId);

  const endpoint = new URL('https://cdn.syndication.twimg.com/tweet-result');
  endpoint.searchParams.set('id', statusId);
  endpoint.searchParams.set('lang', resolveLanguage());
  endpoint.searchParams.set('token', '1');

  const payload = await fetchJsonWithTimeout<SyndicationTweetPayload & Record<string, unknown>>(endpoint.toString());

  if (!payload || typeof payload !== 'object') {
    return null;
  }

  const variants = dedupeVariants(collectSyndicationVariants(payload));

  if (variants.length === 0) {
    const referencedIds = collectReferencedStatusIds(payload).filter((id) => id !== statusId);

    for (const referencedId of referencedIds) {
      const resolvedFromReference = await resolveViaSyndication(referencedId, {
        depth: options.depth + 1,
        visited: options.visited,
      });

      if (resolvedFromReference) {
        return resolvedFromReference;
      }
    }

    const fallbackVariants = dedupeVariants(collectVariantsRecursively(payload));
    if (fallbackVariants.length === 0) {
      return null;
    }

    const fallbackMp4Variants = fallbackVariants
      .filter((variant) => variant.contentType.includes('video/mp4'))
      .sort((a, b) => b.bitrate - a.bitrate);

    const fallbackPicked =
      fallbackMp4Variants[0] ||
      fallbackVariants.find((variant) => variant.contentType.includes('mpegurl')) ||
      fallbackVariants[0];

    if (!fallbackPicked?.url) {
      return null;
    }

    return {
      url: fallbackPicked.url,
      mime: fallbackPicked.contentType || inferMime(fallbackPicked.url),
      isVertical: resolveVerticalFromSyndication(payload),
      sourceStatusId: fallbackPicked.sourceStatusId || statusId,
    };
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
    sourceStatusId: picked.sourceStatusId || statusId,
  };
};

const extractVideoUrlFromHtml = (html: string) => {
  const candidates = html.match(/https:\/\/video\.twimg\.com\/[^"'\\\s<>()]+/gi) || [];

  for (const candidate of candidates) {
    const cleaned = decodeHtmlAttribute(candidate);
    if (!cleaned) {
      continue;
    }

    if (cleaned.includes('.mp4') || cleaned.includes('.m3u8')) {
      return cleaned;
    }
  }

  return null;
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
    extractMetaContent(html, 'twitter:player:stream') ||
    extractVideoUrlFromHtml(html);

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
    sourceStatusId: statusId,
  };
};
