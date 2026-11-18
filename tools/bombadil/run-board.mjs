// Explores the overlay board with Bombadil through the Tauri shell mock.
//
// Serves apps/lc-overlay over HTTP (board-mock.html loads ../web/app/main.js)
// and opens dev/board-mock.html?media, so the media editor gets a real video
// and a real sound. Nothing reaches the network: the mock answers every shell
// command and server call.
//
// Environment: LC_BOMBADIL_BOARD_PORT (39418), BOMBADIL_TIME_LIMIT (3m),
// BOMBADIL_CMD, BOMBADIL_NO_SANDBOX (see lib.mjs).

import { createReadStream, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, sep } from 'node:path';
import { here, runBombadil } from './lib.mjs';

const port = Number(process.env.LC_BOMBADIL_BOARD_PORT ?? 39418);
const root = join(here, '..', '..', 'apps', 'lc-overlay');

const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mp3': 'audio/mpeg',
  '.ogg': 'audio/ogg',
  '.wav': 'audio/wav',
  '.woff2': 'font/woff2',
};

const server = createServer((request, response) => {
  const path = decodeURIComponent(new URL(request.url ?? '/', 'http://localhost').pathname);
  const file = normalize(join(root, path));
  let size = 0;
  try {
    if (!file.startsWith(root + sep)) {
      throw new Error('outside root');
    }
    const stat = statSync(file);
    if (!stat.isFile()) {
      throw new Error('not a file');
    }
    size = stat.size;
  } catch {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, {
    'Content-Type': types[extname(file).toLowerCase()] ?? 'application/octet-stream',
    'Content-Length': size,
    'Cache-Control': 'no-store',
  });
  createReadStream(file).pipe(response);
});

await new Promise((resolve) => server.listen(port, '127.0.0.1', resolve));

const base = `http://127.0.0.1:${port}`;
const code = await runBombadil({
  origin: `${base}/dev/board-mock.html?media`,
  spec: 'specs/board.ts',
  output: 'out/board',
  // Keeps the explorer on the board: the other dev pages are not under test.
  allowUrl: `${base}/dev/board-mock.html*`,
});
server.close();
process.exit(code);
