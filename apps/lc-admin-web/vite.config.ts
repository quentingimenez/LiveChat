import { defineConfig } from 'vitest/config';
import preact from '@preact/preset-vite';

// The console is served by lc-server under /admin/, so every generated asset
// URL must carry that prefix. The API paths in src/api/client.ts are absolute
// (/admin/api/…) for the same reason.
export default defineConfig({
  base: '/admin/',
  plugins: [preact()],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
    css: false,
    restoreMocks: true,
    // The Windows sandbox used by the agent cannot spawn the default forks
    // pool; worker threads run in-process and behave the same for this suite.
    pool: 'threads',
    maxWorkers: 1,
    minWorkers: 1,
  },
});
