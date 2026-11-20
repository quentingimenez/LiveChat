#!/usr/bin/env node
/**
 * Vérifie que le build produit est chargeable par Chrome : chaque content script
 * déclaré dans le manifeste doit exister et être un script *classique*.
 *
 * `new Function` analyse la source exactement comme un script classique (et non
 * comme un module) : la moindre instruction ESM lève la même erreur que Chrome,
 * ce qui attrape la régression « plus aucun bouton injecté » avant la release.
 */
import { access, readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = resolve(root, 'dist');

const manifestPath = resolve(dist, 'manifest.json');
const manifest = JSON.parse(await readFile(manifestPath, 'utf8').catch(() => {
  console.error(`✗ ${manifestPath} introuvable : lance d'abord \`npm run build\`.`);
  process.exit(1);
}));

const contentScripts = (manifest.content_scripts ?? []).flatMap((entry) => entry.js ?? []);

if (contentScripts.length === 0) {
  console.error('✗ manifest.json : aucun content script déclaré.');
  process.exit(1);
}

let failures = 0;

for (const file of contentScripts) {
  const path = resolve(dist, file);

  try {
    await access(path);
  } catch {
    console.error(`✗ ${file} : fichier absent du build.`);
    failures += 1;
    continue;
  }

  const source = await readFile(path, 'utf8');

  try {
    new Function(source);
    console.log(`✓ ${file} : script classique`);
  } catch (error) {
    console.error(`✗ ${file} : ${error instanceof Error ? error.message : String(error)}`);
    failures += 1;
  }
}

// Le thème clair est posé avant le premier rendu par un script classique : le
// miroir doit être livré et référencé par les deux pages, sinon le popup et les
// options démarreraient en sombre malgré la préférence enregistrée.
try {
  await access(resolve(dist, 'theme-guard.js'));
  console.log('✓ theme-guard.js : livré');
} catch {
  console.error('✗ theme-guard.js : fichier absent du build.');
  failures += 1;
}

for (const page of ['popup.html', 'options.html']) {
  const source = await readFile(resolve(dist, page), 'utf8').catch(() => '');

  if (!source.includes('theme-guard.js')) {
    console.error(`✗ ${page} : ne référence pas theme-guard.js.`);
    failures += 1;
    continue;
  }

  console.log(`✓ ${page} : référence theme-guard.js`);
}

process.exit(failures === 0 ? 0 : 1);
