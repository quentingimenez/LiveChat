import { resolve } from 'node:path';
import { defineConfig } from 'vite';

export default defineConfig({
  publicDir: 'public',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        background: resolve(__dirname, 'src/background/index.ts'),
        'youtube-content': resolve(__dirname, 'src/content/youtube.ts'),
        'tiktok-content': resolve(__dirname, 'src/content/tiktok.ts'),
        'twitter-content': resolve(__dirname, 'src/content/twitter.ts'),
        popup: resolve(__dirname, 'popup.html'),
        options: resolve(__dirname, 'options.html')
      },
      output: {
        entryFileNames: 'assets/[name].js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name][extname]'
      }
    }
  }
});
