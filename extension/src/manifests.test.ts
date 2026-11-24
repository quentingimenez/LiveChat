import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Garde de sécurité C-05 : ces tests épinglent la surface d'accès réseau de
// l'extension. Tout élargissement des permissions d'hôte exige une revue de
// sécurité explicite et la mise à jour de la documentation (README, décision
// C-05). Ne pas assouplir ces assertions pour faire passer un build.

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const readJson = async (name: string): Promise<Record<string, unknown>> =>
  JSON.parse(await readFile(resolve(root, 'manifests', name), 'utf8')) as Record<string, unknown>;

const MEDIA_HOSTS = [
  'https://youtube.com/*',
  'https://*.youtube.com/*',
  'https://youtu.be/*',
  'https://tiktok.com/*',
  'https://*.tiktok.com/*',
  'https://x.com/*',
  'https://*.x.com/*',
  'https://twitter.com/*',
  'https://*.twitter.com/*',
];

const WIDE_PATTERNS = ['<all_urls>', '*://*/*', 'http://*/*', 'https://*'];
const SECURITY_REVIEW = 'élargir les permissions d’hôte exige une revue de sécurité';

describe('invariants des manifestes (décision C-05)', () => {
  it('ne déclare qu’un seul motif d’hôte optionnel, volontairement large en déclaratif', async () => {
    const base = await readJson('base.json');

    expect(base.optional_host_permissions, SECURITY_REVIEW).toEqual(['https://*/*']);
  });

  it('ne déclare que les 9 hôtes média en permissions d’hôte statiques', async () => {
    const base = await readJson('base.json');

    expect(base.host_permissions, SECURITY_REVIEW).toEqual(MEDIA_HOSTS);
  });

  it('ne déclare que les 4 permissions API nécessaires', async () => {
    const base = await readJson('base.json');

    expect(base.permissions, SECURITY_REVIEW).toEqual(['storage', 'contextMenus', 'tabs', 'webRequest']);
  });

  it('ne contient aucun motif d’hôte large dans base, chromium ni firefox', async () => {
    const manifests = await Promise.all(['base.json', 'chromium.json', 'firefox.json'].map(readJson));

    for (const [index, manifest] of manifests.entries()) {
      const serialized = JSON.stringify(manifest);

      for (const pattern of WIDE_PATTERNS) {
        // "https://*" est interdit sauf sous la forme exacte "https://*/*" :
        // c'est le motif optionnel déclaratif retenu en C-05.
        const forbidden =
          pattern === 'https://*'
            ? serialized.includes('"https://*"')
            : serialized.includes(pattern);

        expect(forbidden, `${['base', 'chromium', 'firefox'][index]}: ${SECURITY_REVIEW} (motif ${pattern})`).toBe(
          false,
        );
      }
    }
  });
});
