import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { mergeManifest } from '../../scripts/lib/manifest.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

const readJson = async (path: string) => JSON.parse(await readFile(path, 'utf8'));

describe('mergeManifest', () => {
  it('injecte la version de package.json dans les deux manifestes', async () => {
    const [base, chromium, firefox, packageJson] = await Promise.all([
      readJson(resolve(root, 'manifests/base.json')),
      readJson(resolve(root, 'manifests/chromium.json')),
      readJson(resolve(root, 'manifests/firefox.json')),
      readJson(resolve(root, 'package.json'))
    ]);

    expect(mergeManifest(base, chromium, packageJson.version).version).toBe(packageJson.version);
    expect(mergeManifest(base, firefox, packageJson.version).version).toBe(packageJson.version);
  });

  it('garde le service worker et le minimum Chrome pour Chromium', async () => {
    const [base, chromium] = await Promise.all([
      readJson(resolve(root, 'manifests/base.json')),
      readJson(resolve(root, 'manifests/chromium.json'))
    ]);

    const manifest = mergeManifest(base, chromium, '1.2.3');

    expect(manifest.background).toEqual({
      service_worker: 'assets/background.js',
      type: 'module'
    });
    expect(manifest.minimum_chrome_version).toBe('148');
    expect(manifest.browser_specific_settings).toBeUndefined();
  });

  it('remplace le background Chromium par background.scripts pour Firefox', async () => {
    const [base, chromium, firefox] = await Promise.all([
      readJson(resolve(root, 'manifests/base.json')),
      readJson(resolve(root, 'manifests/chromium.json')),
      readJson(resolve(root, 'manifests/firefox.json'))
    ]);

    // Le manifeste Chromium peut être fusionné d'abord : la surcouche Firefox
    // doit remplacer entièrement le background, pas le compléter.
    const manifest = mergeManifest(mergeManifest(base, chromium, '1.2.3'), firefox, '1.2.3');

    expect(manifest.background).toEqual({
      scripts: ['assets/background.js'],
      type: 'module'
    });
    expect(manifest.background.service_worker).toBeUndefined();
    expect(manifest.browser_specific_settings.gecko.id).toBe('livechat-extension@livechat');
    expect(manifest.browser_specific_settings.gecko.strict_min_version).toBe('140.0');
    expect(manifest.browser_specific_settings.gecko.data_collection_permissions.required).toEqual([
      'browsingActivity',
      'authenticationInfo'
    ]);
  });

  it('ne déclare pas update_url : AMO met à jour les add-ons listés', async () => {
    const [base, firefox] = await Promise.all([
      readJson(resolve(root, 'manifests/base.json')),
      readJson(resolve(root, 'manifests/firefox.json'))
    ]);

    const manifest = mergeManifest(base, firefox, '1.2.3');

    expect(manifest.browser_specific_settings.gecko.update_url).toBeUndefined();
  });

  it('remplace les tableaux au lieu de les concaténer', () => {
    const merged = mergeManifest(
      { permissions: ['storage'], content_scripts: [{ js: ['a.js'] }] },
      { permissions: ['tabs'], content_scripts: [{ js: ['b.js'] }] },
      '1.0.0'
    );

    expect(merged.permissions).toEqual(['tabs']);
    expect(merged.content_scripts).toEqual([{ js: ['b.js'] }]);
  });

  it('ne mute pas les manifestes source', () => {
    const base = { name: 'LiveChat', nested: { value: 1 } };
    const override = { nested: { value: 2 } };

    mergeManifest(base, override, '9.9.9');

    expect(base).toEqual({ name: 'LiveChat', nested: { value: 1 } });
  });
});
