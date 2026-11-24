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

const HOST_PERMISSION_KEYS = ['host_permissions', 'optional_host_permissions'] as const;
const MANIFEST_NAMES = ['base', 'chromium', 'firefox'] as const;
// Motifs grossiers interdits partout, y compris sous forme de chaîne ailleurs
// dans le fichier. Le guillemet fermant sur `https://*` vise le motif nu, sans
// le `/*` final du motif déclaratif retenu en C-05.
const WIDE_PATTERNS = ['<all_urls>', '*://*/*', 'http://*/*', '"https://*"'];
const DECLARATIVE_PATTERN = 'https://*/*';
const SECURITY_REVIEW = 'élargir les permissions d’hôte exige une revue de sécurité';

const declaredHostPatterns = (manifest: Record<string, unknown>): string[] =>
  HOST_PERMISSION_KEYS.flatMap((key) => {
    const value = manifest[key];
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
  });

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

  it('ne laisse aucune surcouche déclarer d’hôtes supplémentaires', async () => {
    for (const name of ['chromium', 'firefox']) {
      const overlay = await readJson(`${name}.json`);

      for (const key of HOST_PERMISSION_KEYS) {
        const value = overlay[key];
        expect(
          value === undefined || (Array.isArray(value) && value.length === 0),
          `${name}.json: ${SECURITY_REVIEW} (${key} interdit dans une surcouche)`,
        ).toBe(true);
      }
    }
  });

  it('ne contient aucun motif d’hôte large dans base, chromium ni firefox', async () => {
    const manifests = await Promise.all(MANIFEST_NAMES.map((name) => readJson(`${name}.json`)));

    for (const [index, manifest] of manifests.entries()) {
      const serialized = JSON.stringify(manifest);
      const name = MANIFEST_NAMES[index];

      for (const pattern of WIDE_PATTERNS) {
        expect(
          serialized.includes(pattern),
          `${name}.json: ${SECURITY_REVIEW} (motif ${pattern})`,
        ).toBe(false);
      }

      // `https://*/*` est le motif déclaratif retenu, autorisé uniquement dans
      // optional_host_permissions de base.json. Partout ailleurs (hôtes
      // statiques, surcouches), il doit faire échouer le test.
      for (const pattern of declaredHostPatterns(manifest)) {
        const optional = manifest.optional_host_permissions;
        const allowed = name === 'base' && Array.isArray(optional) && optional[0] === pattern;
        expect(
          allowed || pattern !== DECLARATIVE_PATTERN,
          `${name}.json: ${SECURITY_REVIEW} (motif ${DECLARATIVE_PATTERN} hors base.optional_host_permissions)`,
        ).toBe(true);
      }
    }
  });
});
