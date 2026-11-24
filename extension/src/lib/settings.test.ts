import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_AUTHOR_NAME,
  ensureApiPermissionTransition,
  hasApiHostPermission,
  normalizeSettingsInput,
  requestApiHostPermission,
} from './settings';

describe('settings validation', () => {
  it('normalise une config valide', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://livechat.example.com/',
      ingestToken: ' abc ',
      guildId: '123',
      authorName: 'Mon extension',
      authorImage: 'https://cdn.example.com/avatar.png',
    });

    expect(result.ok).toBe(true);

    if (result.ok) {
      expect(result.value.apiUrl).toBe('https://livechat.example.com');
      expect(result.value.ingestToken).toBe('abc');
      expect(result.value.guildId).toBe('123');
      expect(result.value.authorName).toBe('Mon extension');
      expect(result.value.authorImage).toBe('https://cdn.example.com/avatar.png');
    }
  });

  it('applique un authorName par défaut si vide', () => {
    const result = normalizeSettingsInput({
      apiUrl: 'https://livechat.example.com',
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

describe('rétrécissement de la demande de permission d’hôte (C-05)', () => {
  const permissionsRequest = vi.fn(async (permissions: { origins: string[] }) => permissions.origins.length > 0);
  const permissionsContains = vi.fn(async (permissions: { origins: string[] }) => permissions.origins.length === 0);
  const permissionsRemove = vi.fn(async () => false);
  const storageGet = vi.fn(async () => ({}));

  beforeEach(() => {
    vi.clearAllMocks();
    permissionsRequest.mockImplementation(async () => true);
    permissionsContains.mockImplementation(async () => false);
    permissionsRemove.mockImplementation(async () => false);
    storageGet.mockImplementation(async () => ({}));

    const storageArea = {
      get: storageGet,
      set: vi.fn(async () => undefined),
      remove: vi.fn(async () => undefined),
      clear: vi.fn(async () => undefined),
    };

    vi.stubGlobal('browser', {
      permissions: {
        contains: permissionsContains,
        request: permissionsRequest,
        remove: permissionsRemove,
      },
      storage: {
        local: storageArea,
        session: storageArea,
        onChanged: { addListener: vi.fn(), removeListener: vi.fn() },
      },
      runtime: { lastError: undefined },
    });
  });

  it('demande uniquement l’origine configurée, pas le motif déclaratif large', async () => {
    await expect(requestApiHostPermission('https://board.example.com')).resolves.toBe(true);

    expect(permissionsRequest).toHaveBeenCalledTimes(1);
    expect(permissionsRequest).toHaveBeenCalledWith({ origins: ['https://board.example.com/*'] });
    expect(permissionsRequest.mock.calls[0][0].origins).not.toContain('https://*/*');
  });

  it('ignore chemin, query et slash final : seule l’origine est demandée', async () => {
    await expect(
      ensureApiPermissionTransition(' HTTPS://Board.Example.Com:8443/ingest?token=abc/ '),
    ).resolves.toMatchObject({ granted: true, pattern: 'https://board.example.com:8443/*' });

    expect(permissionsRequest).toHaveBeenCalledTimes(1);
    expect(permissionsRequest).toHaveBeenCalledWith({ origins: ['https://board.example.com:8443/*'] });
    expect(permissionsRequest.mock.calls[0][0].origins).not.toContain('https://*/*');
  });

  it('ne fait aucune demande pour une URL API en http (refus de normalizeApiUrl)', async () => {
    await expect(requestApiHostPermission('http://board.example.com')).rejects.toThrow(
      'URL API invalide. Utilise https://.',
    );
    await expect(ensureApiPermissionTransition('http://localhost:3000')).rejects.toThrow(
      'URL API invalide. Utilise https://.',
    );

    expect(permissionsRequest).not.toHaveBeenCalled();
  });

  it('ne demande rien de plus quand l’autorisation est déjà accordée : contains cible la même origine', async () => {
    // Les fonctions de demande restent idempotentes (Firefox impose le geste
    // utilisateur) : aucune ne court-circuite `request`. L'invariant vérifiable
    // est que la lecture d'état `hasApiHostPermission` ne sonde QUE l'origine
    // configurée — jamais le motif déclaratif large — et qu'aucune demande
    // n'est émise sur ce chemin de vérification.
    permissionsContains.mockImplementationOnce(async () => true);

    await expect(hasApiHostPermission('https://board.example.com')).resolves.toBe(true);

    expect(permissionsContains).toHaveBeenCalledTimes(1);
    expect(permissionsContains).toHaveBeenCalledWith({ origins: ['https://board.example.com/*'] });
    expect(permissionsContains.mock.calls[0][0].origins).not.toContain('https://*/*');
    expect(permissionsRequest).not.toHaveBeenCalled();
  });
});
