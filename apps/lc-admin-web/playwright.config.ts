// End-to-end tests of the admin console against the real lc-server binary.
//
// `e2e/server.mjs` starts the binary with a fresh SQLite database in a temp
// folder, the secret login enabled and the built console embedded. Build both
// first: `npm run build` here, then `cargo build -p lc-server` (the binary
// embeds apps/lc-admin-web/dist at compile time).
//
// LC_SERVER_BIN overrides the binary path (default: <repo>/target/debug/lc-server).

import { defineConfig, devices } from '@playwright/test';

export const E2E_PORT = Number(process.env.LC_E2E_PORT ?? 39_317);
export const E2E_SECRET = 'e2e-admin-secret-0123456789abcdef';

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  timeout: 30_000,
  use: {
    baseURL: `http://127.0.0.1:${E2E_PORT}`,
    trace: 'retain-on-failure',
    locale: 'fr-FR',
  },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node e2e/server.mjs',
    url: `http://127.0.0.1:${E2E_PORT}/health`,
    reuseExistingServer: false,
    timeout: 60_000,
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      LC_E2E_PORT: String(E2E_PORT),
      LC_E2E_SECRET: E2E_SECRET,
    },
  },
});
