// Starts the real lc-server binary for the end-to-end tests: fresh SQLite
// database and media folder in a temp directory, secret login only, no
// Discord bot. The temp directory is removed when the process stops.

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../../..');
const exe = process.platform === 'win32' ? 'lc-server.exe' : 'lc-server';
const binary = process.env.LC_SERVER_BIN ?? join(repo, 'target', 'debug', exe);

if (!existsSync(binary)) {
  console.error(`lc-server binary not found: ${binary} (build it or set LC_SERVER_BIN)`);
  process.exit(1);
}

const port = process.env.LC_E2E_PORT ?? '39317';
const dir = mkdtempSync(join(tmpdir(), 'lc-admin-e2e-'));
const databasePath = join(dir, 'livechat.sqlite').replaceAll('\\', '/');

const child = spawn(binary, [], {
  stdio: 'inherit',
  env: {
    ...process.env,
    // `development` exempts loopback clients from the rate limits; the
    // Bombadil runner (tools/bombadil) needs it, as it clicks far faster
    // than a person.
    NODE_ENV: process.env.LC_E2E_NODE_ENV ?? 'test',
    PORT: port,
    API_URL: `http://127.0.0.1:${port}`,
    DATABASE_URL: `sqlite://${databasePath}`,
    MEDIA_STORAGE_DIR: join(dir, 'media'),
    MEDIA_URL_SECRET: 'e2e-media-url-secret',
    ADMIN_UI_TOKEN: process.env.LC_E2E_SECRET ?? 'e2e-admin-secret-0123456789abcdef',
    ADMIN_UI_LOCAL_ONLY: 'true',
    INGEST_API_TOKEN: 'e2e-ingest-token',
    DISCORD_TOKEN: '',
    RUST_LOG: process.env.RUST_LOG ?? 'warn,lc_server::admin_audit=info',
  },
});

function stop() {
  if (!child.killed) {
    child.kill();
  }
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    stop();
    process.exit(0);
  });
}

child.on('exit', (code) => {
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {
    // The database may still be locked on Windows; the OS cleans temp later.
  }
  process.exit(code ?? 0);
});
