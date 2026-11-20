#!/usr/bin/env node
/**
 * Build de l'extension LiveChat.
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
 */
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = resolve(root, 'dist');
const watch = process.argv.includes('--watch');

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

/**
 * Background service worker (`"type": "module"`), popup et options.
 * `emptyOutDir` n'est activé que pour le build complet : en veille, vider `dist`
 * effacerait les fichiers écrits par les trois autres passes.
 */
const appConfig = (emptyOutDir) => ({
  configFile: false,
  root,
  publicDir: resolve(root, 'public'),
  build: {
    outDir,
    emptyOutDir,
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
const contentScriptConfig = (name, entry) => ({
  configFile: false,
  root,
  publicDir: false,
  build: {
    outDir,
    emptyOutDir: false,
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

const contentScriptConfigs = () =>
  Object.entries(CONTENT_SCRIPTS).map(([name, entry]) => contentScriptConfig(name, entry));

const buildAll = async (emptyOutDir) => {
  for (const config of [appConfig(emptyOutDir), ...contentScriptConfigs()]) {
    await build(config);
  }
};

const watchAll = async () => {
  for (const config of [appConfig(false), ...contentScriptConfigs()]) {
    await build({ ...config, build: { ...config.build, watch: {} } });
  }
};

try {
  // Un build complet d'abord : en veille, les quatre passes démarrent en
  // parallèle et `dist` doit déjà contenir tous les fichiers. Les veilleurs
  // n'effacent ensuite jamais `dist` (emptyOutDir désactivé).
  await buildAll(true);

  if (watch) {
    await watchAll();
    console.log('Veille active sur dist/ — Ctrl+C pour arrêter.');
  }
} catch (error) {
  console.error(`Build de l'extension échoué : ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
