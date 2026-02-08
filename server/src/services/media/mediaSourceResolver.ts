import crypto from 'crypto';

export interface ResolvedMediaSource {
  sourceUrl: string;
  sourceHash: string;
}

const pickSource = (url?: string | null, media?: string | null) => {
  const candidate = (media || url || '').toString().trim();

  if (!candidate) {
    return null;
  }

  return candidate;
};

const normalizeYoutubeUrl = (url: URL): URL => {
  const normalized = new URL(url.toString());

  if (normalized.hostname === 'youtu.be') {
    const videoId = normalized.pathname.replace('/', '').trim();
    normalized.hostname = 'www.youtube.com';
    normalized.pathname = '/watch';
    normalized.search = '';
    if (videoId) {
      normalized.searchParams.set('v', videoId);
    }
  }

  const videoId = normalized.searchParams.get('v');
  const startAt = normalized.searchParams.get('t');

  normalized.search = '';

  if (videoId) {
    normalized.searchParams.set('v', videoId);
  }

  if (startAt) {
    normalized.searchParams.set('t', startAt);
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

export const resolveMediaSource = (params: {
  url?: string | null;
  media?: string | null;
}): ResolvedMediaSource | null => {
  const sourceUrl = pickSource(params.url, params.media);

  if (!sourceUrl) {
    return null;
  }

  const canonicalSource = canonicalizeSourceUrl(sourceUrl);

  return {
    sourceUrl: canonicalSource,
    sourceHash: buildSourceHash(canonicalSource),
  };
};
