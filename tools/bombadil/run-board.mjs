// Explores the overlay board with Bombadil through the Tauri shell mock.
//
// Serves apps/lc-overlay over HTTP (board-mock.html loads ../web/app/main.js)
// and opens dev/board-mock.html?media, so the media editor gets a real video
// and a real sound. Nothing reaches the network: the mock answers every shell
// command and server call.
//
// Environment: LC_BOMBADIL_BOARD_PORT (39418), LC_BOMBADIL_BOARD_QUERY (the
// mock options, `media`; e.g. `role=guest&media` or `empty`), BOMBADIL_SPEC
// (`specs/board.ts`), BOMBADIL_OUTPUT (`out/board`), BOMBADIL_TIME_LIMIT (3m),
// BOMBADIL_CMD, BOMBADIL_NO_SANDBOX (see lib.mjs).

import { join } from 'node:path';
import { here, runBombadil } from './lib.mjs';
import { createStaticServer } from './static-server.mjs';

const port = Number(process.env.LC_BOMBADIL_BOARD_PORT ?? 39418);
const root = join(here, '..', '..', 'apps', 'lc-overlay');

const server = createStaticServer({ root });

await new Promise((resolve) => server.listen(port, '127.0.0.1', resolve));

const base = `http://127.0.0.1:${port}`;
const query = process.env.LC_BOMBADIL_BOARD_QUERY ?? 'media';
const code = await runBombadil({
  origin: `${base}/dev/board-mock.html${query ? `?${query}` : ''}`,
  spec: process.env.BOMBADIL_SPEC ?? 'specs/board.ts',
  output: process.env.BOMBADIL_OUTPUT ?? 'out/board',
  // Keeps the explorer on the board: the other dev pages are not under test.
  allowUrl: `${base}/dev/board-mock.html*`,
});
server.close();
process.exit(code);
