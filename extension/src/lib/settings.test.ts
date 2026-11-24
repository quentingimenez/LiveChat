import { afterEach, describe, expect, it, vi } from 'vitest';
import { getSettings, normalizeSettingsInput } from './settings';

describe('settings validation', () => {
  it('normalise une config valide', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://livechat.example.com/',
      ingestToken: ' abc ',
      guildId: '123',
    });

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.value).toEqual({
        apiUrl: 'https://livechat.example.com',
        ingestToken: 'abc',
        guildId: '123',
      });
    }
  });

  it('ignore les anciens champs auteur', () => {
    const legacy = {
      apiUrl: 'https://livechat.example.com',
      ingestToken: 'token',
      guildId: '123',
      authorName: 'Ancien pseudo',
      authorImage: 'data:image/png;base64,AAAA',
    };

    const result = normalizeSettingsInput(legacy);

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.value).toEqual({
        apiUrl: 'https://livechat.example.com',
        ingestToken: 'token',
        guildId: '123',
      });
    }
  });

  it('échoue sur une API_URL http', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'http://livechat.example.com',
      ingestToken: 'token',
      guildId: '123',
    });

    expect(result.ok).toBe(false);

    if (!result.ok) {
      expect(result.message).toBe('URL API invalide. Utilise https://.');
    }
  });

  it('refuse une API_URL http même localhost', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'http://localhost:3000',
      ingestToken: 'token',
      guildId: '123',
    });

    expect(result.ok).toBe(false);

    if (!result.ok) {
      expect(result.message).toBe('URL API invalide. Utilise https://.');
    }
  });

  it('échoue sans token', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://livechat.example.com',
      guildId: '123',
      ingestToken: '',
    });

    expect(result.ok).toBe(false);

    if (!result.ok) {
      expect(result.message).toContain('INGEST_API_TOKEN');
    }
  });
});

describe('getSettings', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const stubStorage = (stored: unknown) => {
    const set = vi.fn(async () => undefined);
    vi.stubGlobal('browser', {
      storage: {
        local: {
          get: vi.fn(async () => ({ 'lce.settings.v1': stored })),
          set,
        },
      },
    });
    return set;
  };

  it('efface les champs auteur hérités du stockage', async () => {
    const set = stubStorage({
      apiUrl: 'https://livechat.example.com',
      ingestToken: 'token',
      guildId: '123',
      authorName: 'Ancien pseudo',
      authorImage: 'data:image/png;base64,AAAA',
    });

    const settings = await getSettings();

    expect(settings).toEqual({
      apiUrl: 'https://livechat.example.com',
      ingestToken: 'token',
      guildId: '123',
    });
    expect(set).toHaveBeenCalledTimes(1);
    expect(set).toHaveBeenCalledWith({
      'lce.settings.v1': {
        apiUrl: 'https://livechat.example.com',
        ingestToken: 'token',
        guildId: '123',
      },
    });
  });

  it('ne réécrit pas un stockage déjà propre', async () => {
    const set = stubStorage({
      apiUrl: 'https://livechat.example.com',
      ingestToken: 'token',
      guildId: '123',
    });

    await getSettings();

    expect(set).not.toHaveBeenCalled();
  });
});
