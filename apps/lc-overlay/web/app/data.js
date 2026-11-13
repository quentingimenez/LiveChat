// Loading and refreshing guild data (guild info, board items, queue, admin
// lists) and applying the shell's guild list.

import { api, run } from './core/api.js';
import { mediaRefreshDelay } from './core/format.js';
import { invoke } from './core/ipc.js';
import { activeGuild, can, state } from './core/state.js';
import { notify } from './core/toast.js';
import { renderHeader } from './views/header.js';
import { renderOnAir } from './views/queue.js';
import { renderRail } from './views/rail.js';
import { renderView } from './views/view.js';

let guildRefreshTimer = null;
let queueEndTimer = null;
let itemsReloadTimer = null;
let itemsReloadPromise = null;
let itemsReloadResolve = null;
let itemsRefreshTimer = null;
// A media URL expired while the window was hidden: reload it on the next
// visibility/focus instead of fetching in the background.
let itemsRefreshPending = false;

// Board changes are coalesced: nearby requests share one debounced loadItems()
// and the same promise.
export function requestItemsReload() {
  if (itemsReloadPromise) {
    clearTimeout(itemsReloadTimer);
    itemsReloadTimer = setTimeout(runItemsReload, 300);
    return itemsReloadPromise;
  }
  itemsReloadPromise = new Promise((resolve) => {
    itemsReloadResolve = resolve;
  });
  itemsReloadTimer = setTimeout(runItemsReload, 300);
  return itemsReloadPromise;
}

function runItemsReload() {
  itemsReloadTimer = null;
  const resolve = itemsReloadResolve;
  itemsReloadPromise = null;
  itemsReloadResolve = null;
  loadItems()
    .catch(() => {})
    .finally(() => {
      if (resolve) {
        resolve();
      }
    });
}

function guildStatus(guild) {
  return guild && guild.connection ? guild.connection.status : null;
}

// Gives the server time to notice the closed WebSocket before reading the
// guild's connection state again.
function scheduleGuildRefresh() {
  clearTimeout(guildRefreshTimer);
  guildRefreshTimer = setTimeout(loadGuild, 800);
}

export function clearQueueEndTimer() {
  clearTimeout(queueEndTimer);
  queueEndTimer = null;
}

export function applyGuilds(payload) {
  if (!payload) {
    return;
  }
  const previous = state.activeGuildId;
  const previousGuild = activeGuild();
  const previousStatus = previousGuild && previousGuild.connection ? previousGuild.connection.status : null;
  const previousStatuses = new Map();
  for (const guild of state.guilds) {
    if (guild && guild.membership) {
      previousStatuses.set(guild.membership.guildId, guildStatus(guild));
    }
  }
  state.guilds = payload.guilds || [];
  state.activeGuildId = payload.activeGuildId || null;
  state.profile = payload.profile || state.profile;
  for (const guild of state.guilds) {
    if (!guild || !guild.membership) {
      continue;
    }
    const guildId = guild.membership.guildId;
    if (guildStatus(guild) === 'connected' && previousStatuses.get(guildId) !== 'connected') {
      document.dispatchEvent(new CustomEvent('livechat:guild-connected', { detail: { guildId } }));
    }
  }
  document.dispatchEvent(new CustomEvent('livechat:guilds-updated'));
  if (previous !== state.activeGuildId) {
    clearTimeout(guildRefreshTimer);
    guildRefreshTimer = null;
    clearQueueEndTimer();
    resetGuildData();
    loadGuild();
  } else if (state.activeGuildId) {
    const guild = activeGuild();
    const status = guild && guild.connection ? guild.connection.status : null;
    if (previousStatus !== status) {
      scheduleGuildRefresh();
    }
  }
  renderRail();
  renderHeader();
  if (!state.activeGuildId || state.page === 'settings') {
    renderView();
  }
}

function resetGuildData() {
  state.info = null;
  state.items = [];
  state.itemsLoaded = false;
  state.selectedId = null;
  state.queue = { playing: null, pending: [], at: 0 };
  state.queueStale = false;
  state.triggers.clear();
  state.triggerHints.clear();
  state.members = [];
  state.invites = [];
  state.search = '';
  state.kind = 'all';
  clearTimeout(itemsRefreshTimer);
  itemsRefreshTimer = null;
  itemsRefreshPending = false;
  if (state.tab === 'admin') {
    state.tab = 'board';
  }
}

export async function loadGuildInfo() {
  if (!state.activeGuildId) {
    return false;
  }
  const guildId = state.activeGuildId;
  try {
    const info = await api('GET', '/overlay/guild');
    if (guildId !== state.activeGuildId) {
      return false;
    }
    state.info = info;
  } catch (error) {
    if (guildId !== state.activeGuildId) {
      return false;
    }
    notify(error.message, true);
  }
  renderHeader();
  return true;
}

export async function loadGuild() {
  if (!(await loadGuildInfo())) {
    return;
  }
  const bindings = await invoke('media_board_get_bindings').catch(() => null);
  state.bindings = (bindings && bindings.bindings) || {};
  renderHeader();
  await Promise.all([loadItems(), refreshQueueAutomatically()]);
}

export async function loadItems() {
  if (!state.activeGuildId) {
    return;
  }
  const guildId = state.activeGuildId;
  try {
    const body = await api('GET', '/overlay/media-board/items?limit=200');
    if (guildId !== state.activeGuildId) {
      return;
    }
    state.items = body.items || [];
  } catch (error) {
    state.items = [];
    if (error.code !== 'forbidden') {
      notify(error.message, true);
    }
  }
  state.itemsLoaded = true;
  scheduleItemsRefresh();
  if (state.tab === 'board' && state.page === 'guild') {
    renderView();
  }
}

// Signed media URLs expire about an hour after they are issued: reload the
// board five minutes before the earliest expiry. While the window is hidden
// (board in the tray), wait for the next visibilitychange or focus.
function scheduleItemsRefresh() {
  clearTimeout(itemsRefreshTimer);
  itemsRefreshTimer = null;
  itemsRefreshPending = false;
  const delay = mediaRefreshDelay(state.items);
  if (delay === null) {
    return;
  }
  itemsRefreshTimer = setTimeout(() => {
    itemsRefreshTimer = null;
    if (document.hidden) {
      itemsRefreshPending = true;
      return;
    }
    requestItemsReload();
  }, delay);
}

export function reloadItemsWhenVisible() {
  if (!itemsRefreshPending || document.hidden || !state.activeGuildId) {
    return;
  }
  itemsRefreshPending = false;
  requestItemsReload();
}

// Automatic queue refreshes are useless while the page is hidden (board in the
// tray): mark the queue stale and read it once when the page comes back.
export function refreshQueueAutomatically() {
  if (document.hidden) {
    state.queueStale = true;
    return;
  }
  loadQueue();
}

function scheduleQueueEndRefresh() {
  clearQueueEndTimer();
  const playing = state.queue.playing;
  // Nothing playing: no end-of-media refresh to schedule.
  if (!playing) {
    return;
  }
  const started = playing.startedAt ? new Date(playing.startedAt).getTime() : NaN;
  if (!Number.isFinite(started) || !(playing.durationSec > 0)) {
    return;
  }
  const endsAt = started + playing.durationSec * 1000;
  const delay = Math.max(0, endsAt - Date.now()) + 1500;
  queueEndTimer = setTimeout(refreshQueueAutomatically, delay);
}

export async function loadQueue() {
  if (!state.activeGuildId || document.hidden) {
    return;
  }
  const guildId = state.activeGuildId;
  try {
    const body = await api('GET', '/overlay/queue');
    if (guildId !== state.activeGuildId) {
      return;
    }
    state.queue = { playing: body.playing || null, pending: body.pending || [], at: Date.now() };
    state.queueStale = false;
    // Board cards listen to this event to update their playback badges without
    // re-rendering the view (the detail panel may hold a playing video).
    document.dispatchEvent(new CustomEvent('livechat:queue'));
  } catch {
    return;
  }
  renderHeader();
  renderOnAir();
  if (state.tab === 'queue' && state.page === 'guild') {
    renderView();
  }
  scheduleQueueEndRefresh();
}

export async function loadAdmin() {
  if (state.adminTab === 'members' && can('manageMembers')) {
    const body = await run(() => api('GET', '/overlay/admin/members'));
    state.members = (body && body.items) || [];
  }
  if (state.adminTab === 'invites' && can('manageInvites')) {
    const body = await run(() => api('GET', '/overlay/admin/invites'));
    state.invites = (body && body.items) || [];
  }
  if (state.tab === 'admin' && state.page === 'guild') {
    renderView();
  }
}
