import fetch from 'node-fetch';
import { MediaIngestionError } from '../media/mediaErrors';
import type { TweetCardPayload } from '../messages/richOverlayPayload';

const TWITTER_HOSTS = new Set(['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com', 'mobile.twitter.com']);
const TWEET_STATUS_PATH_REGEX = /^\/([a-zA-Z0-9_]{1,25})\/status\/(\d+)/;
const TWEET_URL_REGEX =
  /https?:\/\/(?:www\.)?(?:x\.com|twitter\.com|mobile\.twitter\.com)\/[a-zA-Z0-9_]{1,25}\/status\/\d+/i;
const getTwitterOEmbedTimeoutMs = () => {
  const configuredTimeout =
    typeof env?.MEDIA_DOWNLOAD_TIMEOUT_MS === 'number' && Number.isFinite(env.MEDIA_DOWNLOAD_TIMEOUT_MS)
      ? env.MEDIA_DOWNLOAD_TIMEOUT_MS
      : 5000;
  return Math.min(12000, Math.max(5000, configuredTimeout));
};

const sanitizeEmbedHtml = (rawHtml: string) => {
  return rawHtml
    .replace(/<script[\s\S]*?<\/script>/gi, '')
    .replace(/<iframe[\s\S]*?<\/iframe>/gi, '')
    .trim();
};

const resolveLanguage = () => {
  const normalized = (env.I18N || 'en').toLowerCase().trim();
  return normalized.startsWith('fr') ? 'fr' : 'en';
};

const fetchJsonWithTimeout = async (url: string) => {
  const controller = new AbortController();
  const timeoutMs = getTwitterOEmbedTimeoutMs();
  const timeout = setTimeout(() => {
    controller.abort();
  }, timeoutMs);

  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: {
        'user-agent': 'LiveChatBot/1.0 (+https://github.com/WardenPro/LiveChat-Bot)',
      },
    });

    if (!response.ok) {
      throw new MediaIngestionError(
        'DOWNLOAD_FAILED',
        `Unable to resolve tweet embed (${response.status})`,
        `Twitter oEmbed failed with status ${response.status} for ${url}`,
      );
    }

    return response.json();
  } catch (error) {
    if ((error as { name?: string })?.name === 'AbortError') {
      throw new MediaIngestionError('DOWNLOAD_TIMEOUT', 'Tweet embed request timed out');
    }

    if (error instanceof MediaIngestionError) {
      throw error;
    }

    throw new MediaIngestionError('DOWNLOAD_FAILED', 'Tweet embed request failed');
  } finally {
    clearTimeout(timeout);
  }
};

export const normalizeTweetStatusUrl = (rawUrl: string): string | null => {
  const directCandidate = `${rawUrl || ''}`.trim();
  const extractedCandidate = directCandidate.match(TWEET_URL_REGEX)?.[0] || directCandidate;
  const candidate = extractedCandidate.trim();

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);
    const host = parsed.hostname.toLowerCase();

    if (!TWITTER_HOSTS.has(host)) {
      return null;
    }

    const pathMatch = parsed.pathname.match(TWEET_STATUS_PATH_REGEX);

    if (!pathMatch) {
      return null;
    }

    const [, handle, statusId] = pathMatch;
    return `https://x.com/${handle}/status/${statusId}`;
  } catch {
    return null;
  }
};

export const resolveTweetCardFromUrl = async (rawUrl?: string | null): Promise<TweetCardPayload | null> => {
  return resolveTweetCardFromUrlWithOptions(rawUrl, {
    hideMedia: false,
  });
};

export const resolveTweetCardFromUrlWithOptions = async (
  rawUrl?: string | null,
  options?: {
    hideMedia?: boolean;
  },
): Promise<TweetCardPayload | null> => {
  if (!rawUrl) {
    return null;
  }

  const normalizedTweetUrl = normalizeTweetStatusUrl(rawUrl);

  if (!normalizedTweetUrl) {
    return null;
  }

  const oEmbedUrl = new URL(env.TWITTER_OEMBED_ENDPOINT);
  oEmbedUrl.searchParams.set('url', normalizedTweetUrl);
  oEmbedUrl.searchParams.set('omit_script', 'true');
  oEmbedUrl.searchParams.set('dnt', 'true');
  oEmbedUrl.searchParams.set('align', 'center');
  oEmbedUrl.searchParams.set('hide_thread', 'false');
  oEmbedUrl.searchParams.set('hide_media', options?.hideMedia ? 'true' : 'false');
  oEmbedUrl.searchParams.set('lang', resolveLanguage());

  const payload = (await fetchJsonWithTimeout(oEmbedUrl.toString())) as {
    html?: unknown;
    author_name?: unknown;
    author_url?: unknown;
  };

  if (typeof payload?.html !== 'string' || payload.html.trim() === '') {
    throw new MediaIngestionError('INVALID_MEDIA', 'Tweet embed payload is invalid');
  }

  const html = sanitizeEmbedHtml(payload.html);

  if (!html) {
    throw new MediaIngestionError('INVALID_MEDIA', 'Tweet embed HTML is empty');
  }

  return {
    url: normalizedTweetUrl,
    html,
    authorName: typeof payload?.author_name === 'string' ? payload.author_name.trim() : 'Tweet',
    authorUrl: typeof payload?.author_url === 'string' ? payload.author_url.trim() : null,
  };
};
