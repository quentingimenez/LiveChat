const HTTP_PROTOCOLS = new Set(['http:', 'https:']);
const YOUTUBE_HOSTS = new Set(['www.youtube.com', 'youtube.com', 'm.youtube.com', 'youtu.be']);
const TWITTER_HOSTS = new Set(['x.com', 'www.x.com', 'twitter.com', 'www.twitter.com']);
const isTikTokHost = (hostname: string): boolean => hostname === 'tiktok.com' || hostname.endsWith('.tiktok.com');

const toUrl = (raw: string, base?: string): URL | null => {
  try {
    return new URL(raw, base);
  } catch {
    return null;
  }
};

const trimAndNormalize = (value: string): string => value.trim();

const normalizeOriginBase = (url: URL): string => {
  const pathWithoutTrailingSlash = url.pathname.replace(/\/+$/, '');
  const path = pathWithoutTrailingSlash === '/' ? '' : pathWithoutTrailingSlash;
  return `${url.origin}${path}${url.search}`;
};

export const normalizeApiUrl = (rawUrl: string): string => {
  const candidate = trimAndNormalize(rawUrl);
  const parsed = toUrl(candidate);

  if (!parsed || !HTTP_PROTOCOLS.has(parsed.protocol)) {
    throw new Error('URL API invalide. Utilise http:// ou https://.');
  }

  parsed.hash = '';
  return normalizeOriginBase(parsed);
};

export const toApiOriginPattern = (apiUrl: string): string => {
  const normalized = normalizeApiUrl(apiUrl);
  const parsed = new URL(normalized);
  return `${parsed.origin}/*`;
};

export const normalizeGenericHttpUrl = (rawUrl: string, base?: string): string | null => {
  const parsed = toUrl(rawUrl.trim(), base);

  if (!parsed || !HTTP_PROTOCOLS.has(parsed.protocol)) {
    return null;
  }

  parsed.hash = '';
  return parsed.toString();
};

export const normalizeYoutubeUrl = (rawUrl: string, base?: string): string | null => {
  const parsed = toUrl(rawUrl.trim(), base);

  if (!parsed || !YOUTUBE_HOSTS.has(parsed.hostname.toLowerCase())) {
    return null;
  }

  if (parsed.hostname.toLowerCase() === 'youtu.be') {
    const shortId = parsed.pathname.replace(/^\//, '').trim();

    if (!shortId) {
      return null;
    }

    const watchUrl = new URL('https://www.youtube.com/watch');
    watchUrl.searchParams.set('v', shortId);
    return watchUrl.toString();
  }

  if (parsed.pathname.startsWith('/shorts/')) {
    const shortId = parsed.pathname.split('/').filter(Boolean)[1];

    if (!shortId) {
      return null;
    }

    return `https://www.youtube.com/shorts/${shortId}`;
  }

  const videoId = parsed.searchParams.get('v');

  if (!videoId) {
    return null;
  }

  const watchUrl = new URL('https://www.youtube.com/watch');
  watchUrl.searchParams.set('v', videoId);

  const timestamp = parsed.searchParams.get('t');
  if (timestamp) {
    watchUrl.searchParams.set('t', timestamp);
  }

  return watchUrl.toString();
};

export const normalizeTikTokVideoUrl = (rawUrl: string, base?: string): string | null => {
  const parsed = toUrl(rawUrl.trim(), base);

  if (!parsed || !isTikTokHost(parsed.hostname.toLowerCase())) {
    return null;
  }

  const namedMediaMatch = parsed.pathname.match(/^\/@([^/]+)\/(video|photo)\/(\d{15,22})(?:\/|$)/i);
  if (namedMediaMatch) {
    return `https://www.tiktok.com/@${namedMediaMatch[1]}/${namedMediaMatch[2].toLowerCase()}/${namedMediaMatch[3]}`;
  }

  const genericMediaMatch = parsed.pathname.match(/^\/(video|photo)\/(\d{15,22})(?:\/|$)/i);
  if (genericMediaMatch) {
    return `https://www.tiktok.com/${genericMediaMatch[1].toLowerCase()}/${genericMediaMatch[2]}`;
  }

  return null;
};

export const normalizeTwitterStatusUrl = (rawUrl: string, base?: string): string | null => {
  const parsed = toUrl(rawUrl.trim(), base);

  if (!parsed || !TWITTER_HOSTS.has(parsed.hostname.toLowerCase())) {
    return null;
  }

  const userStatusMatch = parsed.pathname.match(/^\/([^/]+)\/status\/(\d+)/i);
  if (userStatusMatch) {
    return `${parsed.origin}/${userStatusMatch[1]}/status/${userStatusMatch[2]}`;
  }

  const webStatusMatch = parsed.pathname.match(/^\/i\/web\/status\/(\d+)/i);
  if (webStatusMatch) {
    return `${parsed.origin}/i/web/status/${webStatusMatch[1]}`;
  }

  return null;
};

export const resolveIngestTargetUrl = (rawUrl: string, base?: string): string | null => {
  const candidate = trimAndNormalize(rawUrl);

  if (!candidate) {
    return null;
  }

  const parsed = toUrl(candidate, base);
  if (!parsed) {
    return null;
  }

  const hostname = parsed.hostname.toLowerCase();

  if (YOUTUBE_HOSTS.has(hostname)) {
    return normalizeYoutubeUrl(parsed.toString(), base);
  }

  if (isTikTokHost(hostname)) {
    return normalizeTikTokVideoUrl(parsed.toString(), base) || normalizeGenericHttpUrl(parsed.toString());
  }

  if (TWITTER_HOSTS.has(hostname)) {
    return normalizeTwitterStatusUrl(parsed.toString(), base);
  }

  return normalizeGenericHttpUrl(parsed.toString());
};

interface ContextUrlCandidates {
  linkUrl?: string;
  srcUrl?: string;
  pageUrl?: string;
  tabUrl?: string;
}

export const resolveUrlFromContextCandidates = (candidates: ContextUrlCandidates): string | null => {
  const orderedCandidates = [candidates.linkUrl, candidates.srcUrl, candidates.pageUrl, candidates.tabUrl].filter(
    (value): value is string => typeof value === 'string' && value.trim().length > 0,
  );

  for (const candidate of orderedCandidates) {
    const normalized = resolveIngestTargetUrl(candidate);
    if (normalized) {
      return normalized;
    }
  }

  return null;
};
