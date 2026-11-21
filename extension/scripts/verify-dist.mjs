#!/usr/bin/env node
/**
 * Vérifie que chaque build produit est chargeable par son navigateur.
 *
 * Chromium : chaque content script déclaré doit exister et être un script
 * *classique* (`new Function` analyse la source comme Chrome le fait) ; le
 * background doit être un service worker de module.
 *
 * Firefox : le background doit être déclaré via `background.scripts` (Firefox
 * n'exécute pas de service worker dans une extension) et le manifeste doit
 * porter l'identifiant AMO ainsi que le consentement de collecte de données
 * exigé pour toute nouvelle soumission.
 */
import { access, readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const targetArg = process.argv.find((arg) => arg.startsWith('--target='));
const requestedTarget = targetArg?.slice('--target='.length) ?? 'all';
const TARGETS = ['chromium', 'firefox'];
const targets = requestedTarget === 'all' ? TARGETS : [requestedTarget];

const readJson = async (path) => JSON.parse(await readFile(path, 'utf8'));

let failures = 0;

const fail = (message) => {
  console.error(`✗ ${message}`);
  failures += 1;
};

const checkFile = async (path, label) => {
  try {
    await access(path);
    console.log(`✓ ${label}`);
  } catch {
    fail(`${label} : fichier absent du build.`);
  }
};

const checkManifestShape = (target, manifest, version) => {
  if (manifest.version !== version) {
    fail(`${target}/manifest.json : version ${manifest.version} ≠ package.json ${version}`);
  }

  const background = manifest.background ?? {};

  if (target === 'chromium') {
    if (background.service_worker !== 'assets/background.js') {
      fail('chromium/manifest.json : background.service_worker absent.');
    }

    if (background.scripts) {
      fail('chromium/manifest.json : background.scripts ne doit pas être livré à Chromium.');
    }

    if (manifest.minimum_chrome_version !== '148') {
      fail('chromium/manifest.json : minimum_chrome_version doit être 148 (namespace browser).');
    }

    if (manifest.browser_specific_settings) {
      fail('chromium/manifest.json : browser_specific_settings inutile pour Chromium.');
    }
  }

  if (target === 'firefox') {
    const scripts = Array.isArray(background.scripts) ? background.scripts : [];

    if (!scripts.includes('assets/background.js')) {
      fail('firefox/manifest.json : background.scripts doit inclure assets/background.js.');
    }

    if (background.service_worker) {
      fail('firefox/manifest.json : background.service_worker n’est pas supporté par Firefox.');
    }

    if (background.type !== 'module') {
      fail('firefox/manifest.json : background.type doit être module (Firefox 112+).');
    }

    const gecko = manifest.browser_specific_settings?.gecko;

    if (!gecko?.id) {
      fail('firefox/manifest.json : browser_specific_settings.gecko.id manquant (signature AMO).');
    }

    if (!gecko?.strict_min_version) {
      fail('firefox/manifest.json : strict_min_version manquant.');
    }

    // AMO refuse update_url pour un add-on qu'il héberge : les mises à jour
    // passent par ses propres métadonnées de version.
    if (gecko?.update_url) {
      fail('firefox/manifest.json : update_url est interdit pour un add-on listé sur AMO.');
    }

    const required = gecko?.data_collection_permissions?.required ?? [];

    for (const category of ['browsingActivity', 'authenticationInfo']) {
      if (!required.includes(category)) {
        fail(`firefox/manifest.json : data_collection_permissions.required sans ${category}.`);
      }
    }
  }
};

for (const target of targets) {
  if (!TARGETS.includes(target)) {
    fail(`cible inconnue: ${target}`);
    continue;
  }

  const dist = resolve(root, 'dist', target);
  const manifestPath = resolve(dist, 'manifest.json');

  let manifest;

  try {
    manifest = await readJson(manifestPath);
  } catch {
    fail(`${target}/manifest.json introuvable : lance d'abord \`npm run build\`.`);
    continue;
  }

  const packageJson = await readJson(resolve(root, 'package.json'));

  console.log(`\n${target}`);

  checkManifestShape(target, manifest, packageJson.version);

  const contentScripts = (manifest.content_scripts ?? []).flatMap((entry) => entry.js ?? []);

  if (contentScripts.length === 0) {
    fail(`${target}/manifest.json : aucun content script déclaré.`);
  }

  for (const file of contentScripts) {
    const path = resolve(dist, file);

    try {
      await access(path);
    } catch {
      fail(`${target}/${file} : fichier absent du build.`);
      continue;
    }

    const source = await readFile(path, 'utf8');

    try {
      new Function(source);
      console.log(`✓ ${file} : script classique`);
    } catch (error) {
      fail(`${target}/${file} : ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  const backgroundFile = target === 'firefox' ? 'assets/background.js' : manifest.background?.service_worker;

  if (backgroundFile) {
    await checkFile(resolve(dist, backgroundFile), `${target}/${backgroundFile}`);

    // AMO exige une archive de sources pour un paquet minifié. Le build Firefox
    // est volontairement lisible : un bundle minifié tiendrait sur une ligne.
    if (target === 'firefox') {
      const source = await readFile(resolve(dist, backgroundFile), 'utf8').catch(() => '');
      const lines = source.split('\n').length;

      if (source && lines < 20) {
        fail('firefox/assets/background.js : build minifié (AMO exigerait les sources).');
      }
    }
  }

  // Le thème clair est posé avant le premier rendu par un script classique : le
  // miroir doit être livré et référencé par les deux pages, sinon le popup et les
  // options démarreraient en sombre malgré la préférence enregistrée.
  await checkFile(resolve(dist, 'theme-guard.js'), `${target}/theme-guard.js`);

  for (const page of ['popup.html', 'options.html']) {
    const source = await readFile(resolve(dist, page), 'utf8').catch(() => '');

    if (!source.includes('theme-guard.js')) {
      fail(`${target}/${page} : ne référence pas theme-guard.js.`);
      continue;
    }

    console.log(`✓ ${page} : référence theme-guard.js`);
  }
}

process.exit(failures === 0 ? 0 : 1);
