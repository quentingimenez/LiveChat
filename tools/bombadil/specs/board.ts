// Bombadil specification of the overlay board (apps/lc-overlay/web/app),
// explored in a plain browser through the Tauri shell mock
// (apps/lc-overlay/dev/board-mock.html, served by ../run-board.mjs).

import { weighted } from '@antithesishq/bombadil';
import { registerCustomAction } from '@antithesishq/bombadil/browser';
import {
  back,
  clicks,
  inputs,
  reload,
  scroll,
  waitOnce,
} from '@antithesishq/bombadil/browser/defaults/actions';

export {
  noConsoleErrors,
  noHttpErrorCodes,
  noUncaughtExceptions,
  noUnhandledPromiseRejections,
} from '@antithesishq/bombadil/browser/defaults/properties';
export { noLastingBlankPage, noLeakedValues } from './common.ts';

/** The console hooks of board-mock.js (see apps/lc-overlay/dev/README.md). */
type BoardMock = {
  finishPlaying(): void;
  reconnect(guildId: string): void;
  restartServer(): void;
  openSettings(): void;
};

function mock(window: Window): BoardMock | undefined {
  return (window as unknown as { __mock?: BoardMock }).__mock;
}

// Shell and server events the user cannot trigger from the page: they hit the
// code paths that react to the media ending, a dropped WebSocket, a server
// restart and the tray's "Paramètres…" entry.
const finishPlaying = registerCustomAction('finishPlaying', async (_document, window) => {
  mock(window)?.finishPlaying();
});
const reconnect = registerCustomAction('reconnect', async (_document, window) => {
  mock(window)?.reconnect('111');
});
const restartServer = registerCustomAction('restartServer', async (_document, window) => {
  mock(window)?.restartServer();
});
const openSettings = registerCustomAction('openSettings', async (_document, window) => {
  mock(window)?.openSettings();
});

export const boardActions = weighted([
  [10, clicks],
  [4, inputs],
  [2, scroll],
  [1, back],
  [1, reload],
  [1, waitOnce],
  [
    2,
    weighted([
      [1, finishPlaying()],
      [1, reconnect()],
      [1, restartServer()],
      [1, openSettings()],
    ]),
  ],
]);
