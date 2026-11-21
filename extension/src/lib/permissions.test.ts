import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ensureApiPermissionTransition, removePreviousApiHostPermission } from './settings';

/**
 * Firefox n'accorde `permissions.request` que pendant le geste utilisateur :
 * le moindre `await` avant l'appel le fait rejeter avec « may only be called
 * from a user input handler ». Ces tests figent l'ordre des opérations, pas
 * seulement le résultat.
 */
const calls: string[] = [];

const storageGet = vi.fn(async () => {
  calls.push('storage.local.get');
  return {};
});

const storageSet = vi.fn(async () => {
  calls.push('storage.local.set');
});

const permissionsRequest = vi.fn(async () => {
  calls.push('permissions.request');
  return true;
});

const permissionsContains = vi.fn(async () => {
  calls.push('permissions.contains');
  return false;
});

const permissionsRemove = vi.fn(async () => {
  calls.push('permissions.remove');
  return true;
});

beforeEach(() => {
  calls.length = 0;
  vi.clearAllMocks();
  storageGet.mockImplementation(async () => {
    calls.push('storage.local.get');
    return {};
  });
  permissionsRequest.mockImplementation(async () => {
    calls.push('permissions.request');
    return true;
  });
  permissionsRemove.mockImplementation(async () => {
    calls.push('permissions.remove');
    return true;
  });

  const storageArea = {
    get: storageGet,
    set: storageSet,
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
      onChanged: {
        addListener: vi.fn(),
        removeListener: vi.fn(),
      },
    },
    runtime: {
      lastError: undefined,
    },
  });
});

describe('ensureApiPermissionTransition', () => {
  it('demande la permission avant toute lecture du stockage', async () => {
    const result = await ensureApiPermissionTransition('http://localhost:3000');

    expect(result).toMatchObject({ granted: true, pattern: 'http://localhost:3000/*' });
    expect(calls[0]).toBe('permissions.request');
    expect(calls.indexOf('permissions.request')).toBeLessThan(calls.indexOf('storage.local.get'));
  });

  it('ne lit pas le stockage quand la permission est refusée', async () => {
    permissionsRequest.mockImplementationOnce(async () => {
      calls.push('permissions.request');
      return false;
    });

    const result = await ensureApiPermissionTransition('http://localhost:3000');

    expect(result).toMatchObject({ granted: false, reason: 'permission_denied' });
    expect(calls).toEqual(['permissions.request']);
  });

  it('retire l’ancien domaine quand l’URL de l’API change', async () => {
    storageGet.mockImplementationOnce(async () => {
      calls.push('storage.local.get');
      return {
        'lce.settings.v1': {
          apiUrl: 'http://ancien.example:3000',
          ingestToken: 'token',
          guildId: 'guild',
          authorName: 'Auteur',
        },
      };
    });

    const result = await ensureApiPermissionTransition('http://nouveau.example:3000');

    expect(result).toMatchObject({ granted: true, removedPrevious: true });
    expect(permissionsRemove).toHaveBeenCalledWith({ origins: ['http://ancien.example:3000/*'] });
  });

  it('ne retire rien quand le domaine est inchangé', async () => {
    storageGet.mockImplementationOnce(async () => {
      calls.push('storage.local.get');
      return {
        'lce.settings.v1': {
          apiUrl: 'http://localhost:3000',
          ingestToken: 'token',
          guildId: 'guild',
          authorName: 'Auteur',
        },
      };
    });

    const result = await ensureApiPermissionTransition('http://localhost:3000');

    expect(result).toMatchObject({ granted: true, removedPrevious: false });
    expect(permissionsRemove).not.toHaveBeenCalled();
  });
});

describe('removePreviousApiHostPermission', () => {
  it('ne fait rien sans ancienne URL', async () => {
    await expect(removePreviousApiHostPermission(null, 'http://localhost:3000')).resolves.toBe(false);
    expect(permissionsRemove).not.toHaveBeenCalled();
  });

  it('ne fait rien quand les deux URLs partagent la même origine', async () => {
    await expect(
      removePreviousApiHostPermission('http://localhost:3000/ancien', 'http://localhost:3000/nouveau'),
    ).resolves.toBe(false);
    expect(permissionsRemove).not.toHaveBeenCalled();
  });
});
