import { afterEach, describe, expect, it, vi } from 'vitest';
import { restrictLocalStorageToTrustedContexts } from './storage-access';

describe('accès au stockage local', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('restreint storage.local aux contextes de confiance quand setAccessLevel existe', async () => {
    const setAccessLevel = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('browser', {
      storage: {
        local: {
          setAccessLevel,
        },
      },
    });

    await expect(restrictLocalStorageToTrustedContexts()).resolves.toBeUndefined();

    expect(setAccessLevel).toHaveBeenCalledTimes(1);
    expect(setAccessLevel).toHaveBeenCalledWith({ accessLevel: 'TRUSTED_CONTEXTS' });
  });

  it('ne fait rien et ne lance pas d’erreur quand l’API setAccessLevel est absente', async () => {
    vi.stubGlobal('browser', {
      storage: {
        local: {},
      },
    });

    await expect(restrictLocalStorageToTrustedContexts()).resolves.toBeUndefined();
  });

  it('ne fait rien et ne lance pas d’erreur quand browser.storage est absent', async () => {
    vi.stubGlobal('browser', {});

    await expect(restrictLocalStorageToTrustedContexts()).resolves.toBeUndefined();
  });

  it('ne fait pas échouer le démarrage si setAccessLevel rejette', async () => {
    const setAccessLevel = vi.fn().mockRejectedValue(new Error('API indisponible'));
    vi.stubGlobal('browser', {
      storage: {
        local: {
          setAccessLevel,
        },
      },
    });

    await expect(restrictLocalStorageToTrustedContexts()).resolves.toBeUndefined();
  });

  it('ne fait pas échouer le démarrage si setAccessLevel n’est pas une fonction', async () => {
    vi.stubGlobal('browser', {
      storage: {
        local: {
          setAccessLevel: 'TRUSTED_CONTEXTS',
        },
      },
    });

    await expect(restrictLocalStorageToTrustedContexts()).resolves.toBeUndefined();
  });
});
