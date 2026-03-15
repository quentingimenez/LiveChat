/**
 * Pure TikTok URL/capture helper functions extracted from background/index.ts.
 */

import { toNonEmptyString } from './utils';
import { resolveIngestTargetUrl } from './url';

export const TIKTOK_CAPTURE_MAX_ITEMS = 16;

export interface TikTokCaptureRecord {
  itemId: string | null;
  pageUrl: string | null;
  mediaUrl: string | null;
  playUrl: string | null;
  ts: number;
}

export interface TikTokCaptureState {
  activeItemId: string | null;
  latest: TikTokCaptureRecord | null;
  byItemId: Record<string, TikTokCaptureRecord>;
  updatedAt: number;
}

export const isTikTokHostname = (hostname: string): boolean => {
  const normalizedHost = hostname.toLowerCase();
  return normalizedHost === 'tiktok.com' || normalizedHost.endsWith('.tiktok.com');
};

export const normalizeTikTokItemId = (value: unknown): string | null => {
  const normalized = toNonEmptyString(value);

  if (!normalized) {
    return null;
  }

  return /^\d{15,22}$/.test(normalized) ? normalized : null;
};

export const normalizeTikTokPageUrl = (value: unknown): string | null => {
  const candidate = toNonEmptyString(value);

  if (!candidate) {
    return null;
  }

  const normalized = resolveIngestTargetUrl(candidate);

  if (!normalized) {
    return null;
  }

  try {
    const parsed = new URL(normalized);

    if (!isTikTokHostname(parsed.hostname)) {
      return null;
    }

    return /\/(?:video|photo)\/\d{15,22}/i.test(parsed.pathname) ? normalized : null;
  } catch {
    return null;
  }
};

export const normalizeTikTokPlayUrl = (value: unknown): string | null => {
  const candidate = toNonEmptyString(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);

    if (parsed.hostname.toLowerCase() !== 'www.tiktok.com') {
      return null;
    }

    return /^\/aweme\/v100\/play\//i.test(parsed.pathname) ? parsed.toString() : null;
  } catch {
    return null;
  }
};

export const normalizeTikTokMediaUrl = (value: unknown): string | null => {
  const candidate = toNonEmptyString(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);

    if (!parsed.hostname.toLowerCase().includes('tiktok.com')) {
      return null;
    }

    return /\/video\/tos\//i.test(parsed.pathname) ? parsed.toString() : null;
  } catch {
    return null;
  }
};

export const extractTikTokItemIdFromUrl = (value: unknown): string | null => {
  const candidate = toNonEmptyString(value);

  if (!candidate) {
    return null;
  }

  try {
    const parsed = new URL(candidate);
    const pathMatch = parsed.pathname.match(/\/(?:video|photo)\/(\d{15,22})/i);
    const pathItemId = normalizeTikTokItemId(pathMatch?.[1]);

    if (pathItemId) {
      return pathItemId;
    }

    const fromQuery = normalizeTikTokItemId(parsed.searchParams.get('item_id'));

    if (fromQuery) {
      return fromQuery;
    }
  } catch {
    // Ignore malformed URLs.
  }

  const fallbackMatch = candidate.match(/\b(\d{15,22})\b/);
  return normalizeTikTokItemId(fallbackMatch?.[1]);
};

export const createEmptyTikTokCaptureState = (): TikTokCaptureState => ({
  activeItemId: null,
  latest: null,
  byItemId: {},
  updatedAt: Date.now(),
});

export const sanitizeTikTokCaptureRecord = (value: unknown): TikTokCaptureRecord | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const record = value as Record<string, unknown>;
  const itemId = normalizeTikTokItemId(record.itemId);
  const pageUrl = normalizeTikTokPageUrl(record.pageUrl);
  const mediaUrl = normalizeTikTokMediaUrl(record.mediaUrl);
  const playUrl = normalizeTikTokPlayUrl(record.playUrl);
  const ts = typeof record.ts === 'number' && Number.isFinite(record.ts) ? record.ts : Date.now();

  if (!itemId && !pageUrl && !mediaUrl && !playUrl) {
    return null;
  }

  return {
    itemId,
    pageUrl,
    mediaUrl,
    playUrl,
    ts,
  };
};

export const sanitizeTikTokCaptureState = (value: unknown): TikTokCaptureState | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  const state = value as Record<string, unknown>;
  const byItemIdRaw = state.byItemId && typeof state.byItemId === 'object' ? (state.byItemId as Record<string, unknown>) : {};
  const byItemId: Record<string, TikTokCaptureRecord> = {};

  for (const [key, entryRaw] of Object.entries(byItemIdRaw)) {
    const sanitizedEntry = sanitizeTikTokCaptureRecord(entryRaw);

    if (!sanitizedEntry || !sanitizedEntry.itemId) {
      continue;
    }

    byItemId[key] = sanitizedEntry;
  }

  const latest = sanitizeTikTokCaptureRecord(state.latest);
  const activeItemId = normalizeTikTokItemId(state.activeItemId);
  const updatedAt = typeof state.updatedAt === 'number' && Number.isFinite(state.updatedAt) ? state.updatedAt : Date.now();

  return {
    activeItemId,
    latest,
    byItemId,
    updatedAt,
  };
};

export const trimTikTokCaptureItems = (byItemId: Record<string, TikTokCaptureRecord>): Record<string, TikTokCaptureRecord> => {
  const ordered = Object.entries(byItemId)
    .sort(([, left], [, right]) => right.ts - left.ts)
    .slice(0, TIKTOK_CAPTURE_MAX_ITEMS);

  return Object.fromEntries(ordered);
};

export const mergeTikTokCaptureRecord = (
  base: TikTokCaptureRecord | null,
  patch: {
    itemId?: string | null;
    pageUrl?: string | null;
    mediaUrl?: string | null;
    playUrl?: string | null;
  },
): TikTokCaptureRecord | null => {
  const merged: TikTokCaptureRecord = {
    itemId: patch.itemId !== undefined ? patch.itemId : base?.itemId || null,
    pageUrl: patch.pageUrl !== undefined ? patch.pageUrl : base?.pageUrl || null,
    mediaUrl: patch.mediaUrl !== undefined ? patch.mediaUrl : base?.mediaUrl || null,
    playUrl: patch.playUrl !== undefined ? patch.playUrl : base?.playUrl || null,
    ts: Date.now(),
  };

  if (!merged.itemId && !merged.pageUrl && !merged.mediaUrl && !merged.playUrl) {
    return null;
  }

  return merged;
};
