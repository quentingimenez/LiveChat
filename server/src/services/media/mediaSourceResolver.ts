import crypto from 'crypto';
import fetch from 'node-fetch';
import { resolveTweetVideoMediaFromUrl } from '../social/twitterVideoResolver';
import { normalizeTweetStatusUrl } from '../social/twitterOEmbed';

export interface ResolvedMediaSource {
  sourceUrl: string;
  sourceHash: string;
}

const SHORT_URL_RESOLVE_TIMEOUT_MS = 8000;
const TIKTOK_SHORT_HOSTS = new Set(['vm.tiktok.com', 'www.vm.tiktok.com', 'vt.tiktok.com', 'www.vt.tiktok.com']);

const pickSource = (url?: string | null, media?: string | null) => {
  const candidate = (media || url || '').toString().trim();

  if (!candidate) {
    return null;
  }

  return candidate;
};

const parseYouTubeTimestampToSeconds = (rawValue: string): number | null => {
  const value = rawValue.trim().toLowerCase();

  if (!value) {
    return null;
  }

  if (/^\d+$/.test(value)) {
    const parsed = Number.parseInt(value, 10);
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
  }

  if (/^\d+s$/.test(value)) {
    const parsed = Number.parseInt(value.slice(0, -1), 10);
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
  }

  const hmsMatch = value.match(/^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/);
  if (hmsMatch && (hmsMatch[1] || hmsMatch[2] || hmsMatch[3])) {
    const hours = Number.parseInt(hmsMatch[1] || '0', 10);
    const minutes = Number.parseInt(hmsMatch[2] || '0', 10);
    const seconds = Number.parseInt(hmsMatch[3] || '0', 10);
    const totalSeconds = hours * 3600 + minutes * 60 + seconds;
    return Number.isFinite(totalSeconds) && totalSeconds >= 0 ? totalSeconds : null;
  }

  const colonParts = value.split(':').map((part) => part.trim());
  if ((colonParts.length === 2 || colonParts.length === 3) && colonParts.every((part) => /^\d+$/.test(part))) {
    const numericParts = colonParts.map((part) => Number.parseInt(part, 10));
    const [hours, minutes, seconds] = numericParts.length === 3 ? numericParts : [0, numericParts[0], numericParts[1]];
    const totalSeconds = hours * 3600 + minutes * 60 + seconds;
    return Number.isFinite(totalSeconds) && totalSeconds >= 0 ? totalSeconds : null;
  }

  return null;
};

const getYouTubeStartOffsetSecondsFromUrl = (url: URL): number | null => {
  const rawOffset = url.searchParams.get('t') || url.searchParams.get('start');
  const parsedOffsetFromQuery = rawOffset ? parseYouTubeTimestampToSeconds(rawOffset) : null;

  if (parsedOffsetFromQuery !== null && parsedOffsetFromQuery > 0) {
    return parsedOffsetFromQuery;
  }

  if (url.hash) {
    const hashParams = new URLSearchParams(url.hash.replace(/^#/, ''));
    const rawOffsetFromHash = hashParams.get('t') || hashParams.get('start');
    const parsedOffsetFromHash = rawOffsetFromHash ? parseYouTubeTimestampToSeconds(rawOffsetFromHash) : null;
    if (parsedOffsetFromHash !== null && parsedOffsetFromHash > 0) {
      return parsedOffsetFromHash;
    }
  }

  return null;
};

const normalizeYoutubeUrl = (url: URL): URL => {
  const normalized = new URL(url.toString());

  if (normalized.hostname.endsWith('youtube.com')) {
    normalized.hostname = 'www.youtube.com';
  }

  if (normalized.hostname === 'youtu.be') {
    const videoId = normalized.pathname.replace('/', '').trim();
    normalized.hostname = 'www.youtube.com';
    normalized.pathname = '/watch';
    normalized.search = '';
    if (videoId) {
      normalized.searchParams.set('v', videoId);
    }
  }

  const pathnameSegments = normalized.pathname
    .split('/')
    .map((segment) => segment.trim())
    .filter(Boolean);
  const pathVideoId =
    pathnameSegments.length >= 2 && (pathnameSegments[0] === 'shorts' || pathnameSegments[0] === 'embed')
      ? pathnameSegments[1]
      : null;
  const videoId = normalized.searchParams.get('v') || pathVideoId;

  if (videoId) {
    normalized.pathname = '/watch';
  }
  normalized.search = '';

  if (videoId) {
    normalized.searchParams.set('v', videoId);
  }

  normalized.hash = '';

  return normalized;
};

const normalizeTikTokUrl = (url: URL): URL => {
  const normalized = new URL(url.toString());

  normalized.search = '';
  normalized.hash = '';

  return normalized;
};

const isTikTokShortHost = (hostname: string): boolean => {
  return TIKTOK_SHORT_HOSTS.has(hostname.toLowerCase());
};

const resolveUrlWithTimeout = async (rawUrl: string, method: 'HEAD' | 'GET') => {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, SHORT_URL_RESOLVE_TIMEOUT_MS);

  try {
    return await fetch(rawUrl, {
      method,
      redirect: 'follow',
      signal: controller.signal,
      headers: {
        'user-agent':
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36',
      },
    });
  } finally {
    clearTimeout(timeoutId);
  }
};

const resolveShortTikTokUrl = async (rawSource: string): Promise<string> => {
  try {
    const parsed = new URL(rawSource);

    if (!isTikTokShortHost(parsed.hostname)) {
      return rawSource;
    }

    try {
      const headResponse = await resolveUrlWithTimeout(rawSource, 'HEAD');
      if (headResponse.url) {
        try {
          // @ts-ignore Node stream compatibility
          headResponse.body?.destroy?.();
        } catch {
          // ignore cleanup failure
        }
        return headResponse.url;
      }
    } catch {
      // Some origins reject HEAD; fallback to GET
    }

    const getResponse = await resolveUrlWithTimeout(rawSource, 'GET');
    const resolved = getResponse.url || rawSource;
    try {
      // @ts-ignore Node stream compatibility
      getResponse.body?.destroy?.();
    } catch {
      // ignore cleanup failure
    }

    return resolved;
  } catch {
    return rawSource;
  }
};

export const canonicalizeSourceUrl = (rawSource: string): string => {
  const source = rawSource.trim();

  if (!source) {
    return source;
  }

  if (source.startsWith('file:') || source.startsWith('gtts:')) {
    return source;
  }

  try {
    const url = new URL(source);

    if (url.hostname.includes('youtube.com') || url.hostname === 'youtu.be') {
      return normalizeYoutubeUrl(url).toString();
    }

    if (url.hostname.includes('tiktok.com')) {
      return normalizeTikTokUrl(url).toString();
    }

    url.hash = '';

    return url.toString();
  } catch {
    return source;
  }
};

export const buildSourceHash = (source: string): string => {
  return crypto.createHash('sha256').update(source).digest('hex');
};

export const extractMediaStartOffsetSec = (params: { url?: string | null; media?: string | null }): number | null => {
  const sourceUrl = pickSource(params.url, params.media);

  if (!sourceUrl) {
    return null;
  }

  try {
    const parsed = new URL(sourceUrl);
    const isYouTubeSource = parsed.hostname.includes('youtube.com') || parsed.hostname === 'youtu.be';

    if (!isYouTubeSource) {
      return null;
    }

    return getYouTubeStartOffsetSecondsFromUrl(parsed);
  } catch {
    return null;
  }
};

export const resolveMediaSource = async (params: {
  url?: string | null;
  media?: string | null;
}): Promise<ResolvedMediaSource | null> => {
  const sourceUrl = pickSource(params.url, params.media);

  if (!sourceUrl) {
    return null;
  }

  const redirectedSource = await resolveShortTikTokUrl(sourceUrl);
  let canonicalSource = canonicalizeSourceUrl(redirectedSource);

  const normalizedTweetStatusUrl = normalizeTweetStatusUrl(canonicalSource);

  if (normalizedTweetStatusUrl) {
    const resolvedTweetMedia = await resolveTweetVideoMediaFromUrl(normalizedTweetStatusUrl);

    if (resolvedTweetMedia?.url) {
      canonicalSource = canonicalizeSourceUrl(resolvedTweetMedia.url);
    } else {
      canonicalSource = normalizedTweetStatusUrl;
    }
  }

  return {
    sourceUrl: canonicalSource,
    sourceHash: buildSourceHash(canonicalSource),
  };
};
