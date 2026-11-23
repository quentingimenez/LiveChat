// Explores the admin console with Bombadil against the real lc-server binary.
//
// Starts the server through apps/lc-admin-web/e2e/server.mjs (fresh database,
// secret login only, no Discord), logs in with the secret, seeds a few guilds
// so the screens have content, then hands the session cookie to Bombadil.
// Mutations need a same-origin `Origin` header, as a browser would send.
// Build first: `npm run build` in apps/lc-admin-web, then
// `cargo build -p lc-server` (the binary embeds the console at compile time).
//
// Environment: LC_SERVER_BIN (binary path), LC_BOMBADIL_ADMIN_PORT (39417),
// BOMBADIL_SPEC (`specs/admin-console.ts`), BOMBADIL_OUTPUT (`out/admin`),
// BOMBADIL_TIME_LIMIT (3m), BOMBADIL_CMD, BOMBADIL_NO_SANDBOX (see lib.mjs).

import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { here, runBombadil, waitForHttp } from './lib.mjs';

const port = process.env.LC_BOMBADIL_ADMIN_PORT ?? '39417';
const secret = 'bombadil-admin-secret-0123456789abcdef';
const base = `http://127.0.0.1:${port}`;

const server = spawn(process.execPath, [join(here, '..', '..', 'apps', 'lc-admin-web', 'e2e', 'server.mjs')], {
  stdio: 'inherit',
  // `development` lifts the rate limits for loopback clients (see server.mjs).
  // The audit journal would log every click that saves something.
  env: {
    ...process.env,
    LC_E2E_PORT: port,
    LC_E2E_SECRET: secret,
    LC_E2E_NODE_ENV: 'development',
    RUST_LOG: process.env.RUST_LOG ?? 'warn',
  },
});
const serverExited = new Promise((resolve) => server.on('exit', resolve));

async function login() {
  const response = await fetch(`${base}/admin/auth/secret`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Origin: base },
    body: JSON.stringify({ secret }),
  });
  if (response.status !== 204) {
    throw new Error(`secret login answered ${response.status}`);
  }
  const cookie = response.headers.getSetCookie().find((value) => value.startsWith('lc_admin_session='));
  if (!cookie) {
    throw new Error('secret login set no lc_admin_session cookie');
  }
  // `name=value` only: Bombadil's --cookie rejects the SameSite attribute.
  return cookie.split(';')[0];
}

async function seed(session) {
  for (const name of ['Guilde Bombadil', 'Deuxième guilde']) {
    const response = await fetch(`${base}/admin/api/guilds`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Origin: base, Cookie: session },
      body: JSON.stringify({ name }),
    });
    if (!response.ok) {
      throw new Error(`creating guild "${name}" answered ${response.status}`);
    }
  }
}

let code = 1;
try {
  await waitForHttp(`${base}/health`);
  const session = await login();
  await seed(session);
  code = await runBombadil({
    origin: `${base}/admin/`,
    spec: process.env.BOMBADIL_SPEC ?? 'specs/admin-console.ts',
    output: process.env.BOMBADIL_OUTPUT ?? 'out/admin',
    allowUrl: `${base}/admin/*`,
    cookies: [`${session}; Path=/admin; HttpOnly`],
  });
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
} finally {
  server.kill();
  await serverExited;
}
process.exit(code);
