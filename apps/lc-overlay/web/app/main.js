// LiveChat — main application of the overlay (window "board").
//
// ES modules without build step or dependency (CSP `script-src 'self'`). A
// guild token never leaves the Rust shell: every REST call goes through the
// `guild_api` command. No remote data is injected as HTML: the DOM is built
// node by node (`h`).
//
// Layout of `web/app/`:
// - `core/`  : IPC, DOM helpers, shared state, REST, toasts, dialogs,
//              context menus;
// - `data.js`: loading and refreshing guild data;
// - `views/` : rail, header, board, queue, admin, settings and dialogs.

import { installContextMenuGuard } from './core/contextmenu.js';
import { invoke, listen } from './core/ipc.js';
import { state } from './core/state.js';
import { notify } from './core/toast.js';
import {
  applyGuilds,
  clearQueueEndTimer,
  loadGuildInfo,
  loadQueue,
  refreshQueueAutomatically,
  reloadItemsWhenVisible,
  requestItemsReload,
} from './data.js';
import { initGuildRemovalNotices } from './views/notices.js';
import { renderRail } from './views/rail.js';
import { initDeviceSettings, openSettings } from './views/settings.js';
import { initUploadIngestion } from './views/uploads.js';
import { renderView } from './views/view.js';

// No WebView menu (reload, print, save as): the board opens its own menus
// on the rail, the media cards and the queue, and fields keep theirs.
installContextMenuGuard();

listen('overlay://guilds', applyGuilds);

// Device settings: changes made elsewhere and the tray's « Paramètres… » view.
initDeviceSettings();

// Asynchronous media ingestions: conversion results and board resync.
initUploadIngestion();

// Guilds removed by the shell (deleted, member removed, device revoked).
initGuildRemovalNotices();

// The queue is refreshed on a WebSocket hint instead of polling: the server
// limits every route to 100 requests per 15 minutes and per IP.
let queueHintTimer = null;
listen('overlay://queue-hint', (payload) => {
  if (!payload || payload.guildId !== state.activeGuildId) {
    return;
  }
  // The hint already refreshes the queue: the end-of-media refresh scheduled
  // earlier is now redundant.
  clearQueueEndTimer();
  clearTimeout(queueHintTimer);
  queueHintTimer = setTimeout(refreshQueueAutomatically, 500);
});

// A media was added, edited or deleted on the active guild's board (by anyone,
// from the board or from Discord): reload the items once, coalesced.
listen('overlay://board-changed', (payload) => {
  if (payload && payload.guildId === state.activeGuildId) {
    requestItemsReload();
  }
});

// Fallback: a slow refresh only while the queue is really visible and
// focused. No other periodic call to loadQueue.
setInterval(() => {
  if (document.hidden) {
    if (state.tab === 'queue') {
      state.queueStale = true;
    }
    return;
  }
  if (state.tab === 'queue' && document.hasFocus()) {
    refreshQueueAutomatically();
  }
}, 30000);

// Focus and visibility share the same 15 s throttle.
let lastFocusQueueLoad = 0;
function loadQueueOnFocus() {
  const now = Date.now();
  if (state.queueStale) {
    if (document.hidden) {
      return;
    }
    // The queue changed while the page was hidden: read it once.
    state.queueStale = false;
    lastFocusQueueLoad = now;
    loadQueue();
    return;
  }
  if (now - lastFocusQueueLoad < 15000) {
    return;
  }
  lastFocusQueueLoad = now;
  refreshQueueAutomatically();
}

// The count of connected overlays is read at most once every 15 s.
let lastFocusGuildLoad = 0;
function loadGuildOnFocus() {
  if (!state.activeGuildId) {
    return;
  }
  const now = Date.now();
  if (now - lastFocusGuildLoad < 15000) {
    return;
  }
  lastFocusGuildLoad = now;
  loadGuildInfo();
}

window.addEventListener('focus', () => {
  loadGuildOnFocus();
  loadQueueOnFocus();
  reloadItemsWhenVisible();
});
setInterval(() => {
  if (state.page === 'guild' && state.tab === 'queue' && state.queue.playing && !document.hidden) {
    renderView();
  }
}, 1000);
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    loadQueueOnFocus();
    reloadItemsWhenVisible();
  }
});

// The tray stores the view before the board exists: consume it once.
invoke('board_take_pending_view')
  .then((view) => {
    if (view === 'settings') {
      openSettings();
    }
  })
  .catch(() => {});

invoke('guilds_list')
  .then(applyGuilds)
  .catch((error) => {
    renderView();
    notify(String(error.message || error), true);
  });
renderRail();
renderView();
