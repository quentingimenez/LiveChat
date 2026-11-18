// Loading and refreshing guild data (guild info, board items, queue, admin
// lists) and applying the shell's guild list.

import { pruneSelection } from './core/selection.js';
import { api } from './core/api.js';
import { mediaRefreshDelay } from './core/format.js';
import { invoke } from './core/ipc.js';
import { activeGuild, can, state } from './core/state.js';
import { notify } from './core/toast.js';
import { renderHeader } from './views/header.js';
import { renderOnAir } from './views/queue.js';
import { renderRail } from './views/rail.js';
import { renderView } from './views/view.js';

let guildRefreshTimer = null;
let presenceRefreshTimer = null;
// When the guild info was last read, for the focus refresh.
let guildInfoLoadedAt = 0;
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

// Labels of the guild's other connected overlays, as the WebSocket reports
// them: a change means someone connected, muted or left.
function peersKey(guild) {
  const peers = guild && guild.connection && guild.connection.peers;
  return Array.isArray(peers) ? JSON.stringify(peers) : '';
}

function membersShown() {
  return state.page === 'guild' && state.tab === 'admin' && state.adminTab === 'members';
}

function devicesShown() {
  return state.page === 'guild' && state.tab === 'admin' && state.adminTab === 'devices';
}

// Back online: what changed while the socket was closed is read once. The
// delay lets the server register the new connection first.
function scheduleGuildRefresh() {
  clearTimeout(guildRefreshTimer);
  clearTimeout(presenceRefreshTimer);
  presenceRefreshTimer = null;
  guildRefreshTimer = setTimeout(async () => {
    guildRefreshTimer = null;
    await loadGuild({ quiet: true });
    if (membersShown() || devicesShown()) {
      loadAdmin({ quiet: true });
    }
  }, 800);
}

// Only who is online changed (this overlay muted or lost, another one came or
// went): refresh the online count and the members list, never the board. The
// delay gives the server time to notice a closed WebSocket.
function schedulePresenceRefresh() {
  clearTimeout(presenceRefreshTimer);
  presenceRefreshTimer = setTimeout(() => {
    presenceRefreshTimer = null;
    // A reconnection refresh is on its way and reads the same data.
    if (guildRefreshTimer) {
      return;
    }
    loadGuildInfo({ quiet: true });
    if (membersShown() || devicesShown()) {
      loadAdmin({ quiet: true });
    }
  }, 1000);
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
  const previousStatus = guildStatus(previousGuild);
  const previousPeers = peersKey(previousGuild);
  const previousMuted = Boolean(previousGuild && previousGuild.membership.muted);
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
    clearTimeout(presenceRefreshTimer);
    presenceRefreshTimer = null;
    clearQueueEndTimer();
    resetGuildData();
    loadGuild();
  } else if (state.activeGuildId) {
    // Reconnection attempts flip the status often (connecting, error,
    // reconnecting): reloading the whole guild on each flip spent the rate
    // limit and redrew the board for nothing.
    const guild = activeGuild();
    const status = guildStatus(guild);
    const muted = Boolean(guild && guild.membership.muted);
    if (status === 'connected' && previousStatus !== 'connected') {
      scheduleGuildRefresh();
    } else if (
      (previousStatus === 'connected' && status !== 'connected') ||
      peersKey(guild) !== previousPeers ||
      muted !== previousMuted
    ) {
      schedulePresenceRefresh();
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
  state.queueHistory = [];
  state.triggers.clear();
  state.triggerHints.clear();
  state.members = [];
  state.devices = [];
  state.invites = [];
  state.pairingCodes = [];
  state.bans = [];
  state.auditEntries = [];
  state.search = '';
  state.kind = 'all';
  state.selecting = false;
  state.selection = new Set();
  state.selectionAnchor = null;
  clearTimeout(itemsRefreshTimer);
  itemsRefreshTimer = null;
  itemsRefreshPending = false;
  guildInfoLoadedAt = 0;
  if (state.tab === 'admin') {
    state.tab = 'board';
  }
}

export function guildInfoAge() {
  return guildInfoLoadedAt ? Date.now() - guildInfoLoadedAt : Infinity;
}

// `quiet`: a background refresh keeps the last info on failure and shows no
// toast. Each alt-tab used to show the same server error again.
export async function loadGuildInfo(options) {
  if (!state.activeGuildId) {
    return false;
  }
  const quiet = Boolean(options && options.quiet === true);
  const guildId = state.activeGuildId;
  try {
    const info = await api('GET', '/overlay/guild');
    if (guildId !== state.activeGuildId) {
      return false;
    }
    state.info = info;
    guildInfoLoadedAt = Date.now();
  } catch (error) {
    if (guildId !== state.activeGuildId) {
      return false;
    }
    if (!quiet) {
      notify(error.message, true);
    }
  }
  renderHeader();
  return true;
}

export async function loadGuild(options) {
  if (!(await loadGuildInfo(options))) {
    return;
  }
  const bindings = await invoke('media_board_get_bindings').catch(() => null);
  state.bindings = (bindings && bindings.bindings) || {};
  renderHeader();
  await Promise.all([loadItems(options), refreshQueueAutomatically()]);
}

// `options` may be a click event (toolbar button): only `quiet === true`
// counts.
export async function loadItems(options) {
  if (!state.activeGuildId) {
    return;
  }
  const quiet = Boolean(options && options.quiet === true);
  const guildId = state.activeGuildId;
  try {
    const body = await api('GET', '/overlay/media-board/items?limit=200');
    if (guildId !== state.activeGuildId) {
      return;
    }
    state.items = body.items || [];
    state.selection = pruneSelection(state.selection, state.items.map((item) => item.id));
  } catch (error) {
    if (guildId !== state.activeGuildId) {
      return;
    }
    if (error.code === 'forbidden') {
      state.items = [];
    } else if (!quiet) {
      // A transient failure (rate limit, server restarting) keeps the cards
      // already shown instead of emptying the board.
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

// The server may keep a job playing past its duration (an overlay still
// reports it, or the stale-release grace). Each read of an overdue job used to
// schedule the next one 1.5 s later, for as long as the job lasted: the queue
// is now read again after longer and longer pauses, then left to the
// socket's hints.
const OVERDUE_QUEUE_RETRY_MS = [5000, 15000, 60000];
let overdueJobId = null;
let overdueReads = 0;

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
  const remaining = started + playing.durationSec * 1000 - Date.now();
  let delay;
  if (remaining > 0) {
    delay = remaining + 1500;
  } else {
    if (overdueJobId !== playing.id) {
      overdueJobId = playing.id;
      overdueReads = 0;
    }
    if (overdueReads >= OVERDUE_QUEUE_RETRY_MS.length) {
      return;
    }
    delay = OVERDUE_QUEUE_RETRY_MS[overdueReads];
    overdueReads += 1;
  }
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
  // The history only matters on its own tab: reading it on every background
  // queue refresh would spend a request per played media.
  if (state.tab === 'queue' && state.page === 'guild' && can('viewQueue')) {
    try {
      const body = await api('GET', '/overlay/queue/history?limit=30');
      if (guildId === state.activeGuildId) {
        state.queueHistory = body.items || [];
      }
    } catch {
      // A failed history read keeps the list already shown.
    }
  }
  renderHeader();
  renderOnAir();
  if (state.tab === 'queue' && state.page === 'guild') {
    renderView();
  }
  scheduleQueueEndRefresh();
}

// A failed read keeps the list already shown; `quiet` also hides the toast.
async function readList(path, quiet) {
  try {
    const body = await api('GET', path);
    return body.items || [];
  } catch (error) {
    if (!quiet) {
      notify(error.message, true);
    }
    return null;
  }
}

export async function loadAdmin(options) {
  const quiet = Boolean(options && options.quiet === true);
  // Counts of the sub-tabs are known before opening them: members, devices and
  // active codes are read together.
  if (can('manageMembers')) {
    state.members = (await readList('/overlay/admin/members', quiet)) || state.members;
    state.devices = (await readList('/overlay/admin/devices', quiet)) || state.devices;
  }
  if (can('manageInvites')) {
    state.invites = (await readList('/overlay/admin/invites', quiet)) || state.invites;
    state.pairingCodes = (await readList('/overlay/admin/pairing-codes', quiet)) || state.pairingCodes;
  }
  if (state.adminTab === 'bans' && can('manageMembers')) {
    state.bans = (await readList('/overlay/admin/bans', quiet)) || state.bans;
  }
  if (state.adminTab === 'audit' && can('manageSettings')) {
    state.auditEntries = (await readList('/overlay/admin/audit?limit=50', quiet)) || state.auditEntries;
  }
  if (state.tab === 'admin' && state.page === 'guild') {
    renderView();
  }
}
