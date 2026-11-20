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

process.exit(failures === 0 ? 0 : 1);
