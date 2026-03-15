import { describe, expect, it } from 'vitest';
import {
  isTikTokHostname,
  normalizeTikTokItemId,
  normalizeTikTokPageUrl,
  normalizeTikTokPlayUrl,
  normalizeTikTokMediaUrl,
  extractTikTokItemIdFromUrl,
  createEmptyTikTokCaptureState,
  sanitizeTikTokCaptureRecord,
  sanitizeTikTokCaptureState,
  trimTikTokCaptureItems,
  mergeTikTokCaptureRecord,
  TIKTOK_CAPTURE_MAX_ITEMS,
} from './tiktok-capture';

describe('isTikTokHostname', () => {
  it('accepts tiktok.com', () => {
    expect(isTikTokHostname('tiktok.com')).toBe(true);
  });

  it('accepts subdomains of tiktok.com', () => {
    expect(isTikTokHostname('www.tiktok.com')).toBe(true);
    expect(isTikTokHostname('m.tiktok.com')).toBe(true);
    expect(isTikTokHostname('api.tiktok.com')).toBe(true);
  });

  it('is case-insensitive', () => {
    expect(isTikTokHostname('TikTok.com')).toBe(true);
    expect(isTikTokHostname('WWW.TIKTOK.COM')).toBe(true);
  });

  it('rejects non-tiktok hostnames', () => {
    expect(isTikTokHostname('youtube.com')).toBe(false);
    expect(isTikTokHostname('faketiktok.com')).toBe(false);
    expect(isTikTokHostname('nottiktok.com')).toBe(false);
  });
});

describe('normalizeTikTokItemId', () => {
  it('accepts valid 15-22 digit item IDs', () => {
    expect(normalizeTikTokItemId('7591173294007651598')).toBe('7591173294007651598');
    expect(normalizeTikTokItemId('123456789012345')).toBe('123456789012345');
  });

  it('trims whitespace', () => {
    expect(normalizeTikTokItemId('  7591173294007651598  ')).toBe('7591173294007651598');
  });

  it('rejects IDs that are too short', () => {
    expect(normalizeTikTokItemId('12345')).toBeNull();
    expect(normalizeTikTokItemId('12345678901234')).toBeNull();
  });

  it('rejects IDs that are too long', () => {
    expect(normalizeTikTokItemId('12345678901234567890123')).toBeNull();
  });

  it('rejects non-digit strings', () => {
    expect(normalizeTikTokItemId('abc123456789012345')).toBeNull();
  });

  it('rejects non-string values', () => {
    expect(normalizeTikTokItemId(null)).toBeNull();
    expect(normalizeTikTokItemId(undefined)).toBeNull();
    expect(normalizeTikTokItemId(42)).toBeNull();
    expect(normalizeTikTokItemId('')).toBeNull();
  });
});

describe('normalizeTikTokPageUrl', () => {
  it('accepts valid TikTok video URLs', () => {
    const url = 'https://www.tiktok.com/@user/video/7591173294007651598';
    expect(normalizeTikTokPageUrl(url)).toBe(url);
  });

  it('accepts valid TikTok photo URLs', () => {
    const url = 'https://www.tiktok.com/@user/photo/7591173294007651598';
    expect(normalizeTikTokPageUrl(url)).toBe(url);
  });

  it('rejects URLs without video/photo path', () => {
    expect(normalizeTikTokPageUrl('https://www.tiktok.com/@user')).toBeNull();
    expect(normalizeTikTokPageUrl('https://www.tiktok.com/live/12345')).toBeNull();
  });

  it('rejects non-TikTok URLs', () => {
    expect(normalizeTikTokPageUrl('https://www.youtube.com/video/7591173294007651598')).toBeNull();
  });

  it('rejects invalid inputs', () => {
    expect(normalizeTikTokPageUrl(null)).toBeNull();
    expect(normalizeTikTokPageUrl('')).toBeNull();
    expect(normalizeTikTokPageUrl('not a url')).toBeNull();
  });
});

describe('normalizeTikTokPlayUrl', () => {
  it('accepts valid play URLs', () => {
    const url = 'https://www.tiktok.com/aweme/v100/play/?item_id=123456789012345';
    const result = normalizeTikTokPlayUrl(url);
    expect(result).toContain('/aweme/v100/play/');
  });

  it('rejects URLs on wrong hostname', () => {
    expect(normalizeTikTokPlayUrl('https://m.tiktok.com/aweme/v100/play/?item_id=123')).toBeNull();
  });

  it('rejects URLs with wrong path', () => {
    expect(normalizeTikTokPlayUrl('https://www.tiktok.com/other/path')).toBeNull();
  });

  it('rejects invalid inputs', () => {
    expect(normalizeTikTokPlayUrl(null)).toBeNull();
    expect(normalizeTikTokPlayUrl('')).toBeNull();
  });
});

describe('normalizeTikTokMediaUrl', () => {
  it('accepts valid media/tos URLs', () => {
    const url = 'https://v16-webapp.tiktok.com/video/tos/abc123';
    const result = normalizeTikTokMediaUrl(url);
    expect(result).toContain('/video/tos/');
  });

  it('rejects URLs without /video/tos/ path', () => {
    expect(normalizeTikTokMediaUrl('https://www.tiktok.com/other/path')).toBeNull();
  });

  it('rejects non-TikTok domains', () => {
    expect(normalizeTikTokMediaUrl('https://example.com/video/tos/abc')).toBeNull();
  });

  it('rejects invalid inputs', () => {
    expect(normalizeTikTokMediaUrl(null)).toBeNull();
    expect(normalizeTikTokMediaUrl('')).toBeNull();
  });
});

describe('extractTikTokItemIdFromUrl', () => {
  it('extracts ID from /video/ path', () => {
    expect(extractTikTokItemIdFromUrl('https://www.tiktok.com/@user/video/7591173294007651598')).toBe('7591173294007651598');
  });

  it('extracts ID from /photo/ path', () => {
    expect(extractTikTokItemIdFromUrl('https://www.tiktok.com/@user/photo/7591173294007651598')).toBe('7591173294007651598');
  });

  it('extracts ID from item_id query param', () => {
    expect(extractTikTokItemIdFromUrl('https://www.tiktok.com/aweme/v100/play/?item_id=7591173294007651598')).toBe('7591173294007651598');
  });

  it('falls back to matching digits in the URL', () => {
    expect(extractTikTokItemIdFromUrl('https://example.com/7591173294007651598')).toBe('7591173294007651598');
  });

  it('returns null for URLs without item IDs', () => {
    expect(extractTikTokItemIdFromUrl('https://www.tiktok.com/@user')).toBeNull();
  });

  it('returns null for invalid inputs', () => {
    expect(extractTikTokItemIdFromUrl(null)).toBeNull();
    expect(extractTikTokItemIdFromUrl('')).toBeNull();
    expect(extractTikTokItemIdFromUrl(42)).toBeNull();
  });
});

describe('createEmptyTikTokCaptureState', () => {
  it('creates state with null/empty defaults', () => {
    const state = createEmptyTikTokCaptureState();
    expect(state.activeItemId).toBeNull();
    expect(state.latest).toBeNull();
    expect(state.byItemId).toEqual({});
    expect(state.updatedAt).toBeGreaterThan(0);
  });
});

describe('sanitizeTikTokCaptureRecord', () => {
  it('returns null for non-object values', () => {
    expect(sanitizeTikTokCaptureRecord(null)).toBeNull();
    expect(sanitizeTikTokCaptureRecord(undefined)).toBeNull();
    expect(sanitizeTikTokCaptureRecord('string')).toBeNull();
  });

  it('returns null when all fields are empty', () => {
    expect(sanitizeTikTokCaptureRecord({ itemId: null, pageUrl: null })).toBeNull();
  });

  it('normalizes a valid record', () => {
    const result = sanitizeTikTokCaptureRecord({
      itemId: '7591173294007651598',
      pageUrl: 'https://www.tiktok.com/@user/video/7591173294007651598',
      ts: 1000,
    });

    expect(result).not.toBeNull();
    expect(result!.itemId).toBe('7591173294007651598');
    expect(result!.ts).toBe(1000);
  });

  it('uses Date.now() fallback for non-numeric ts', () => {
    const before = Date.now();
    const result = sanitizeTikTokCaptureRecord({
      itemId: '7591173294007651598',
      ts: 'invalid',
    });
    const after = Date.now();

    expect(result).not.toBeNull();
    expect(result!.ts).toBeGreaterThanOrEqual(before);
    expect(result!.ts).toBeLessThanOrEqual(after);
  });
});

describe('sanitizeTikTokCaptureState', () => {
  it('returns null for non-object values', () => {
    expect(sanitizeTikTokCaptureState(null)).toBeNull();
    expect(sanitizeTikTokCaptureState('string')).toBeNull();
  });

  it('sanitizes a valid state object', () => {
    const result = sanitizeTikTokCaptureState({
      activeItemId: '7591173294007651598',
      byItemId: {},
      latest: null,
      updatedAt: 5000,
    });

    expect(result).not.toBeNull();
    expect(result!.activeItemId).toBe('7591173294007651598');
    expect(result!.updatedAt).toBe(5000);
  });

  it('filters invalid entries from byItemId', () => {
    const result = sanitizeTikTokCaptureState({
      byItemId: {
        '7591173294007651598': {
          itemId: '7591173294007651598',
          ts: 1000,
        },
        invalid: {
          itemId: 'not-a-valid-id',
          ts: 2000,
        },
      },
    });

    expect(result).not.toBeNull();
    expect(Object.keys(result!.byItemId)).toEqual(['7591173294007651598']);
  });
});

describe('trimTikTokCaptureItems', () => {
  it('keeps items within the max limit', () => {
    const items: Record<string, { itemId: string; pageUrl: null; mediaUrl: null; playUrl: null; ts: number }> = {};

    for (let i = 0; i < TIKTOK_CAPTURE_MAX_ITEMS + 5; i++) {
      const id = `759117329400765${String(i).padStart(4, '0')}`;
      items[id] = { itemId: id, pageUrl: null, mediaUrl: null, playUrl: null, ts: i };
    }

    const result = trimTikTokCaptureItems(items);
    expect(Object.keys(result).length).toBe(TIKTOK_CAPTURE_MAX_ITEMS);
  });

  it('keeps the most recent items', () => {
    const old = { itemId: '7591173294007651598', pageUrl: null, mediaUrl: null, playUrl: null, ts: 1000 };
    const recent = { itemId: '7591173294007651599', pageUrl: null, mediaUrl: null, playUrl: null, ts: 9000 };

    const result = trimTikTokCaptureItems({
      '7591173294007651598': old,
      '7591173294007651599': recent,
    });

    const entries = Object.entries(result);
    expect(entries[0][1].ts).toBeGreaterThan(entries[1][1].ts);
  });

  it('returns empty for empty input', () => {
    expect(trimTikTokCaptureItems({})).toEqual({});
  });
});

describe('mergeTikTokCaptureRecord', () => {
  it('returns null when all fields are null/undefined', () => {
    expect(mergeTikTokCaptureRecord(null, {})).toBeNull();
  });

  it('merges patch over base', () => {
    const base = {
      itemId: '7591173294007651598',
      pageUrl: 'https://www.tiktok.com/@user/video/7591173294007651598',
      mediaUrl: null,
      playUrl: null,
      ts: 1000,
    };

    const result = mergeTikTokCaptureRecord(base, {
      mediaUrl: 'https://v16.tiktok.com/video/tos/new',
    });

    expect(result).not.toBeNull();
    expect(result!.itemId).toBe('7591173294007651598');
    expect(result!.mediaUrl).toBe('https://v16.tiktok.com/video/tos/new');
    expect(result!.pageUrl).toBe('https://www.tiktok.com/@user/video/7591173294007651598');
  });

  it('patch with undefined keeps base value', () => {
    const base = {
      itemId: '7591173294007651598',
      pageUrl: 'old-url',
      mediaUrl: null,
      playUrl: null,
      ts: 1000,
    };

    const result = mergeTikTokCaptureRecord(base, {
      itemId: '7591173294007651599',
    });

    expect(result).not.toBeNull();
    expect(result!.itemId).toBe('7591173294007651599');
    expect(result!.pageUrl).toBe('old-url');
  });

  it('updates ts to current time', () => {
    const before = Date.now();
    const result = mergeTikTokCaptureRecord(null, {
      itemId: '7591173294007651598',
    });
    const after = Date.now();

    expect(result).not.toBeNull();
    expect(result!.ts).toBeGreaterThanOrEqual(before);
    expect(result!.ts).toBeLessThanOrEqual(after);
  });
});
