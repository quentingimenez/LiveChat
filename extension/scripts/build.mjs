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

/** Background service worker (`"type": "module"`), popup et options. */
const appConfig = {
  configFile: false,
  root,
  publicDir: resolve(root, 'public'),
  build: {
    outDir,
    emptyOutDir: true,
    rollupOptions: {
      input: {
        background: resolve(root, 'src/background/index.ts'),
        popup: resolve(root, 'popup.html'),
        options: resolve(root, 'options.html')
      },
      output: outputNames
    }
  }
};

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

const configs = [appConfig, ...Object.entries(CONTENT_SCRIPTS).map(([name, entry]) => contentScriptConfig(name, entry))];

try {
  for (const config of configs) {
    await build(watch ? { ...config, build: { ...config.build, watch: {} } } : config);
  }
} catch (error) {
  console.error(`Build de l'extension échoué : ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}

if (watch) {
  console.log('Veille active sur dist/ — Ctrl+C pour arrêter.');
}
