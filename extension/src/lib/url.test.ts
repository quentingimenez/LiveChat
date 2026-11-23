import { describe, expect, it } from 'vitest';
import {
  isIngestTargetAllowedForSource,
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
    expect(normalizeApiUrl('https://livechat.exemple.com/')).toBe('https://livechat.exemple.com');
  });

  it('accepte https avec espace, casse et port en conservant le chemin', () => {
    expect(normalizeApiUrl('  HTTPS://LiveChat.Exemple.com:8443/api/  ')).toBe(
      'https://livechat.exemple.com:8443/api',
    );
  });

  it('refuse une URL API http', () => {
    expect(() => normalizeApiUrl('http://livechat.exemple.com')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => normalizeApiUrl('http://livechat.exemple.com:8080/api')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => toApiOriginPattern('http://livechat.exemple.com')).toThrow(
      'URL API invalide. Utilise https://.',
    );
  });

  it('refuse http même sur la boucle locale', () => {
    expect(() => normalizeApiUrl('http://localhost:3000/')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => normalizeApiUrl('http://127.0.0.1:3000')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => normalizeApiUrl('http://[::1]:3000')).toThrow(
      'URL API invalide. Utilise https://.',
    );
  });

  it('refuse une URL http quel que soit l’hôte', () => {
    expect(() => normalizeApiUrl('http://localhost.exemple.com')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => normalizeApiUrl('http://localhost.evil.com:3000')).toThrow(
      'URL API invalide. Utilise https://.',
    );
    expect(() => normalizeApiUrl('http://127.0.0.1.evil.com')).toThrow(
      'URL API invalide. Utilise https://.',
    );
  });

  it('refuse une URL API malformée ou de schéma non http', () => {
    expect(() => normalizeApiUrl('')).toThrow();
    expect(() => normalizeApiUrl('livechat.exemple.com')).toThrow();
    expect(() => normalizeApiUrl('ftp://livechat.exemple.com')).toThrow();
    expect(() => normalizeApiUrl('javascript:alert(1)')).toThrow();
  });

  it('calcule un pattern d’origine pour permissions', () => {
    expect(toApiOriginPattern('https://livechat.exemple.com/api')).toBe('https://livechat.exemple.com/*');
  });

  it('normalise une URL YouTube watch', () => {
    expect(normalizeYoutubeUrl('https://m.youtube.com/watch?v=abc123&t=20')).toBe(
      'https://www.youtube.com/watch?v=abc123&t=20',
    );
  });

  it('normalise une URL YouTube youtu.be en conservant le timestamp', () => {
    expect(normalizeYoutubeUrl('https://youtu.be/abc123?si=test&t=108')).toBe(
      'https://www.youtube.com/watch?v=abc123&t=108',
    );
  });

  it('normalise une URL YouTube watch avec start', () => {
    expect(normalizeYoutubeUrl('https://www.youtube.com/watch?v=abc123&start=42')).toBe(
      'https://www.youtube.com/watch?v=abc123&t=42',
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

  it('canonise un statut twitter.com vers x.com', () => {
    expect(normalizeTwitterStatusUrl('https://www.twitter.com/livechat/status/2020921090097164393')).toBe(
      'https://x.com/livechat/status/2020921090097164393',
    );
  });

  it('accepte un statut X en https avec le port 443', () => {
    expect(normalizeTwitterStatusUrl('https://x.com:443/livechat/status/2020921090097164393')).toBe(
      'https://x.com/livechat/status/2020921090097164393',
    );
  });

  it('rejette un statut X en http, ftp, avec port non standard ou userinfo', () => {
    expect(normalizeTwitterStatusUrl('http://x.com/livechat/status/2020921090097164393')).toBeNull();
    expect(normalizeTwitterStatusUrl('ftp://x.com/livechat/status/2020921090097164393')).toBeNull();
    expect(normalizeTwitterStatusUrl('https://x.com:8443/livechat/status/2020921090097164393')).toBeNull();
    expect(normalizeTwitterStatusUrl('https://user:pass@x.com/livechat/status/2020921090097164393')).toBeNull();
  });

  it('rejette un statut X avec un suffixe ou un schéma non http', () => {
    expect(normalizeTwitterStatusUrl('https://x.com/livechat/status/2020921090097164393abc')).toBeNull();
    expect(normalizeTwitterStatusUrl('javascript:alert(1)')).toBeNull();
  });

  it('accepte un statut X suivi d’un chemin', () => {
    expect(normalizeTwitterStatusUrl('https://x.com/livechat/status/2020921090097164393/photo/1')).toBe(
      'https://x.com/livechat/status/2020921090097164393',
    );
  });

  it('refuse une cible hors de la plateforme déclarée', () => {
    expect(isIngestTargetAllowedForSource('youtube', 'https://x.com/livechat/status/2020921090097164393')).toBe(
      false,
    );
    expect(isIngestTargetAllowedForSource('twitter', 'https://www.youtube.com/watch?v=abc123')).toBe(false);
    expect(isIngestTargetAllowedForSource('tiktok', 'https://www.youtube.com/watch?v=abc123')).toBe(false);
  });

  it('accepte une cible de la plateforme déclarée', () => {
    expect(isIngestTargetAllowedForSource('youtube', 'https://youtu.be/abc123')).toBe(true);
    expect(isIngestTargetAllowedForSource('twitter', 'https://x.com/livechat/status/2020921090097164393')).toBe(
      true,
    );
    expect(isIngestTargetAllowedForSource('tiktok', 'https://www.tiktok.com/@livechat/video/7591173294007651598')).toBe(
      true,
    );
  });

  it('laisse passer le fallback générique sans source de plateforme', () => {
    expect(isIngestTargetAllowedForSource(undefined, 'https://example.com/page')).toBe(true);
    expect(isIngestTargetAllowedForSource('context-menu', 'https://example.com/page')).toBe(true);
    expect(isIngestTargetAllowedForSource('popup', 'https://example.com/page')).toBe(true);
  });

  it('rejette une URL Twitter/X non status', () => {
    expect(resolveIngestTargetUrl('https://x.com/home')).toBeNull();
  });

  it('résout une URL ingest prioritaire', () => {
    expect(resolveIngestTargetUrl('https://youtu.be/abc123')).toBe('https://www.youtube.com/watch?v=abc123');
  });

  it('résout une URL ingest YouTube avec timestamp', () => {
    expect(resolveIngestTargetUrl('https://youtu.be/abc123?t=90')).toBe('https://www.youtube.com/watch?v=abc123&t=90');
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
