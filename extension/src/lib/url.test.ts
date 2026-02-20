import { describe, expect, it } from 'vitest';
import {
  normalizeApiUrl,
  normalizeTikTokVideoUrl,
  normalizeTwitterStatusUrl,
  normalizeYoutubeUrl,
  resolveIngestTargetUrl,
  resolveUrlFromContextCandidates,
  toApiOriginPattern,
} from './url';

describe('url helpers', () => {
  it('normalise une URL API en retirant le slash final', () => {
    expect(normalizeApiUrl('https://bot.exemple.com/')).toBe('https://bot.exemple.com');
  });

  it('calcule un pattern d’origine pour permissions', () => {
    expect(toApiOriginPattern('https://bot.exemple.com/api')).toBe('https://bot.exemple.com/*');
  });

  it('normalise une URL YouTube watch', () => {
    expect(normalizeYoutubeUrl('https://m.youtube.com/watch?v=abc123&t=20')).toBe(
      'https://www.youtube.com/watch?v=abc123&t=20',
    );
  });

  it('normalise une URL YouTube shorts', () => {
    expect(normalizeYoutubeUrl('https://www.youtube.com/shorts/XYZ987?feature=share')).toBe(
      'https://www.youtube.com/shorts/XYZ987',
    );
  });

  it('normalise une URL TikTok vidéo', () => {
    expect(normalizeTikTokVideoUrl('https://www.tiktok.com/@livechat/video/7591173294007651598?foo=bar')).toBe(
      'https://www.tiktok.com/@livechat/video/7591173294007651598',
    );
  });

  it('normalise une URL TikTok photo', () => {
    expect(normalizeTikTokVideoUrl('https://www.tiktok.com/@livechat/photo/7444444444444444444?foo=bar')).toBe(
      'https://www.tiktok.com/@livechat/photo/7444444444444444444',
    );
  });

  it('normalise une URL TikTok générique', () => {
    expect(normalizeTikTokVideoUrl('https://www.tiktok.com/video/7591173294007651598?foo=bar')).toBe(
      'https://www.tiktok.com/video/7591173294007651598',
    );
  });

  it('rejette une URL TikTok avec un id trop court', () => {
    expect(normalizeTikTokVideoUrl('https://www.tiktok.com/video/12345')).toBeNull();
  });

  it('rejette un faux domaine TikTok', () => {
    expect(normalizeTikTokVideoUrl('https://eviltiktok.com/video/7591173294007651598')).toBeNull();
  });

  it('normalise une URL Twitter/X status', () => {
    expect(normalizeTwitterStatusUrl('https://x.com/livechat/status/2020921090097164393?s=20')).toBe(
      'https://x.com/livechat/status/2020921090097164393',
    );
  });

  it('rejette une URL Twitter/X non status', () => {
    expect(resolveIngestTargetUrl('https://x.com/home')).toBeNull();
  });

  it('résout une URL ingest prioritaire', () => {
    expect(resolveIngestTargetUrl('https://youtu.be/abc123')).toBe('https://www.youtube.com/watch?v=abc123');
  });

  it('résout le contexte dans l’ordre link > src > page > tab', () => {
    const resolved = resolveUrlFromContextCandidates({
      pageUrl: 'https://example.com/page',
      tabUrl: 'https://example.com/tab',
      linkUrl: 'https://x.com/livechat/status/2020921090097164393?s=20',
    });

    expect(resolved).toBe('https://x.com/livechat/status/2020921090097164393');
  });

  it('retourne null si le contexte ne contient pas d’URL media supportée', () => {
    const resolved = resolveUrlFromContextCandidates({
      pageUrl: 'https://x.com/home',
      tabUrl: 'https://x.com/home',
    });

    expect(resolved).toBeNull();
  });
});
