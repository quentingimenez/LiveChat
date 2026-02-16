import { describe, expect, it } from 'vitest';
import { DEFAULT_AUTHOR_NAME, normalizeSettingsInput } from './settings';

describe('settings validation', () => {
  it('normalise une config valide', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://bot.example.com/',
      ingestToken: ' abc ',
      guildId: '123',
      authorName: 'Mon extension',
      authorImage: 'https://cdn.example.com/avatar.png',
    });

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.value.apiUrl).toBe('https://bot.example.com');
      expect(result.value.ingestToken).toBe('abc');
      expect(result.value.guildId).toBe('123');
      expect(result.value.authorName).toBe('Mon extension');
      expect(result.value.authorImage).toBe('https://cdn.example.com/avatar.png');
    }
  });

  it('applique un authorName par défaut si vide', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://bot.example.com',
      ingestToken: 'token',
      guildId: '123',
      authorName: '   ',
    });

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.value.authorName).toBe(DEFAULT_AUTHOR_NAME);
      expect(result.value.authorImage).toBeNull();
    }
  });

  it('échoue sans token', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://bot.example.com',
      guildId: '123',
      ingestToken: '',
    });

    expect(result.ok).toBe(false);

    if (!result.ok) {
      expect(result.message).toContain('INGEST_API_TOKEN');
    }
  });
});
