#!/usr/bin/env node
/**
 * Build de l'extension LiveChat, pour Chromium et Firefox.
 *
 * Chrome charge les content scripts comme des scripts classiques : un module ES
 * (le moindre `import`/`export` en tête du fichier) est rejeté avec
 * « Cannot use import statement outside a module » et le script ne s'exécute
 * jamais. C'est ce qui est arrivé quand les trois modales de composition ont
 * partagé `src/lib/duration.ts` : Rollup a extrait un chunk `assets/duration.js`
 * et chaque content script a commencé par `import{...}from"./duration.js"`, donc
 * plus aucun bouton injecté sur X, TikTok et YouTube.
 *
 * Chaque content script est donc bundlé seul, en IIFE auto-suffisante ; le
 * background et les pages (popup, options) restent des modules ES.
 *
 * Le manifeste est assemblé à partir de `manifests/base.json` et de la
 * surcouche de la cible, après les passes Vite (le dossier `public/` ne le
 * contient plus). Sortie : `dist/chromium/` et `dist/firefox/`.
 */
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';
import { mergeManifest } from './lib/manifest.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const watch = process.argv.includes('--watch');
const targetArg = process.argv.find((arg) => arg.startsWith('--target='));
const requestedTarget = targetArg?.slice('--target='.length) ?? 'all';

const TARGETS = ['chromium', 'firefox'];

if (requestedTarget !== 'all' && !TARGETS.includes(requestedTarget)) {
  console.error(`Cible inconnue: ${requestedTarget} (attendu: ${TARGETS.join(', ')} ou all).`);
  process.exit(1);
}

const targets = requestedTarget === 'all' ? TARGETS : [requestedTarget];

const outputNames = {
  entryFileNames: 'assets/[name].js',
  chunkFileNames: 'assets/[name].js',
  assetFileNames: 'assets/[name][extname]'
};

const CONTENT_SCRIPTS = {
  'youtube-content': 'src/content/youtube.ts',
  'tiktok-content': 'src/content/tiktok.ts',
  'twitter-content': 'src/content/twitter.ts'
};

const readJson = async (path) => JSON.parse(await readFile(path, 'utf8'));

const writeManifest = async (target, outDir) => {
  const [base, override, packageJson] = await Promise.all([
    readJson(resolve(root, 'manifests/base.json')),
    readJson(resolve(root, `manifests/${target}.json`)),
    readJson(resolve(root, 'package.json'))
  ]);

  const manifest = mergeManifest(base, override, packageJson.version);
  await writeFile(resolve(outDir, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
};

/**
 * Background (`"type": "module"`), popup et options.
 * `emptyOutDir` n'est activé que pour le build complet : en veille, vider
 * `dist` effacerait les fichiers écrits par les autres passes.
 */
const appConfig = (outDir, emptyOutDir, minify) => ({
  configFile: false,
  root,
  publicDir: resolve(root, 'public'),
  build: {
    outDir,
    emptyOutDir,
    minify,
    rollupOptions: {
      input: {
        background: resolve(root, 'src/background/index.ts'),
        popup: resolve(root, 'popup.html'),
        options: resolve(root, 'options.html')
      },
      output: outputNames
    }
  }
});

/** Un content script = un IIFE autonome, sans chunk partagé ni import. */
const contentScriptConfig = (outDir, name, entry, minify) => ({
  configFile: false,
  root,
  publicDir: false,
  build: {
    outDir,
    emptyOutDir: false,
    minify,
    rollupOptions: {
      input: { [name]: resolve(root, entry) },
      output: {
        ...outputNames,
        format: 'iife',
        inlineDynamicImports: true,
        entryFileNames: `assets/${name}.js`
      }
    }
  }
});

const contentScriptConfigs = (outDir, minify) =>
  Object.entries(CONTENT_SCRIPTS).map(([name, entry]) => contentScriptConfig(outDir, name, entry, minify));

/**
 * Firefox reçoit un build non minifié : AMO exige le code source humainement
 * lisible pour tout paquet minifié ou obfusqué. Un bundle lisible évite
 * d'ajouter une archive de sources à chaque soumission. Chromium garde la
 * minification par défaut de Vite.
 */
const minifyFor = (target) => target !== 'firefox';

const buildTarget = async (target, emptyOutDir) => {
  const outDir = resolve(root, 'dist', target);
  const minify = minifyFor(target);

  for (const config of [appConfig(outDir, emptyOutDir, minify), ...contentScriptConfigs(outDir, minify)]) {
    await build(config);
  }

  await writeManifest(target, outDir);
};

const buildAll = async (emptyOutDir) => {
  for (const target of targets) {
    await buildTarget(target, emptyOutDir);
  }
};

const watchAll = async () => {
  for (const target of targets) {
    const outDir = resolve(root, 'dist', target);
    const minify = minifyFor(target);

    for (const config of [appConfig(outDir, false, minify), ...contentScriptConfigs(outDir, minify)]) {
      await build({ ...config, build: { ...config.build, watch: {} } });
    }
  }
};

try {
  // Un build complet d'abord : en veille, les passes démarrent en parallèle et
  // `dist/<cible>` doit déjà contenir tous les fichiers. Les veilleurs
  // n'effacent ensuite jamais la sortie (emptyOutDir désactivé).
  await buildAll(true);

  if (watch) {
    await watchAll();
    console.log(`Veille active sur ${targets.map((target) => `dist/${target}/`).join(', ')} — Ctrl+C pour arrêter.`);
  }
} catch (error) {
  console.error(`Build de l'extension échoué : ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
