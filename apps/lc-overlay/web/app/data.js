// Loading and refreshing guild data (guild info, board items, queue, admin
// lists) and applying the shell's guild list.

import { apiForGuild } from './core/api.js';
import { readGuildCache, writeGuildCache, invalidateGuildCache, pruneGuildCache } from './core/guild-cache.js';
import { settingsControlActive, settingsFormDirty } from './core/dom.js';
import { syncProfileFromDiscord } from './core/profile-sync.js';
import { pruneSelection } from './core/selection.js';
import { mediaRefreshDelay } from './core/format.js';
import { invoke } from './core/ipc.js';
import { dedupe, isLatest, requestToken } from './core/query.js';
import { activeGuild, can, canAdmin, state } from './core/state.js';
import { tabAfterGuildSwitch } from './core/tabs.js';
import { notify } from './core/toast.js';
import { closeDetail } from './views/board.js';
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
// Guilds whose info is being prefetched (pointer over the rail).
const prefetching = new Set();
// Bumped by forced admin reloads so they cannot join a read that started
// before the mutation they are refreshing.
let adminForcedSeq = 0;
// Token of the newest admin read. Older reads discard their late responses.
let adminReadToken = 0;

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
  pruneGuildCache(state.guilds.map((guild) => guild && guild.membership && guild.membership.guildId).filter(Boolean));
  if (state.pendingGuildId && state.pendingGuildId === state.activeGuildId) {
    state.pendingGuildId = null;
  }
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
    // Paint the new guild from its last snapshot (or a skeleton) at once. The
    // data is revalidated in the background; the old guild's cards never stay
    // clickable under the new guild's identity.
    adoptGuild(state.activeGuildId);
    state.switchingGuild = Boolean(state.activeGuildId);
    // « Administration » depends on permissions we may not have read yet: while
    // the info is missing, trust the role the shell already gave us.
    const admin = state.info ? canAdmin() : Boolean(activeGuild() && activeGuild().membership.role === 'admin');
    state.tab = tabAfterGuildSwitch(state.tab, admin);
    renderRail();
    renderHeader();
    renderView();
    if (state.activeGuildId) {
      hydrateGuild(state.activeGuildId, { switched: true }).catch(() => {});
    }
    return;
  }
  if (state.activeGuildId) {
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
      if (muted !== previousMuted) {
        document.dispatchEvent(new CustomEvent('livechat:guild-mute-changed'));
      }
      schedulePresenceRefresh();
    }
  }
  renderRail();
  // The shell announces one switch twice (its event, then the command's
  // response): the second one repaints the rail and the header without closing
  // the view, so a cached board keeps its scroll and its detail panel.
  renderHeader();
  if (!state.activeGuildId || state.page === 'settings') {
    renderView();
  }
}

function resetGuildData() {
  state.info = null;
  state.infoLoaded = false;
  state.infoError = null;
  state.items = [];
  state.itemsLoaded = false;
  state.itemsError = null;
  // The detail window belongs to the guild being left: close it through the
  // board (data.js <-> board.js is the cycle view.js already creates).
  closeDetail();
  state.queue = { playing: null, pending: [], at: 0 };
  state.queueStale = false;
  state.queueLoaded = false;
  state.queueError = null;
  state.queueHistory = [];
  state.triggers.clear();
  state.triggerHints.clear();
  state.members = [];
  state.devices = [];
  state.invites = [];
  state.pairingCodes = [];
  state.bans = [];
  state.auditEntries = [];
  state.bindings = {};
  state.search = '';
  state.kind = 'all';
  state.selecting = false;
  state.selection = new Set();
  state.selectionAnchor = null;
  clearTimeout(itemsRefreshTimer);
  itemsRefreshTimer = null;
  itemsRefreshPending = false;
  guildInfoLoadedAt = 0;
}

// Restores the last snapshot of `guildId` without touching the network.
function adoptGuild(guildId) {
  if (!guildId) {
    return;
  }
  const info = readGuildCache(guildId, 'info');
  if (info) {
    state.info = info.value;
    state.infoLoaded = true;
    state.infoError = null;
    guildInfoLoadedAt = info.at;
  }
  const items = readGuildCache(guildId, 'items');
  if (items) {
    state.items = items.value || [];
    state.itemsLoaded = true;
    state.itemsError = null;
    state.selection = pruneSelection(state.selection, state.items.map((item) => item.id));
  }
  const queue = readGuildCache(guildId, 'queue');
  if (queue) {
    state.queue = {
      playing: queue.value.playing || null,
      pending: queue.value.pending || [],
      at: queue.at,
    };
    state.queueLoaded = true;
    state.queueError = null;
  }
  const bindings = readGuildCache(guildId, 'bindings');
  if (bindings) {
    state.bindings = bindings.value || {};
  }
  const history = readGuildCache(guildId, 'history');
  if (history) {
    state.queueHistory = history.value || [];
  }
}

export function guildInfoAge() {
  return guildInfoLoadedAt ? Date.now() - guildInfoLoadedAt : Infinity;
}

// Prefetches the info of a guild hovered in the rail: the next switch paints
// its tab set from the cache without waiting for the server.
export function prefetchGuildInfo(guildId) {
  if (!guildId || guildId === state.activeGuildId) {
    return;
  }
  const cached = readGuildCache(guildId, 'info');
  if (cached && cached.fresh) {
    return;
  }
  if (prefetching.has(guildId)) {
    return;
  }
  prefetching.add(guildId);
  apiForGuild(guildId, 'GET', '/overlay/guild', null, { silent: true })
    .then((info) => {
      if (info) {
        writeGuildCache(guildId, 'info', info);
      }
    })
    .catch(() => {})
    .finally(() => {
      prefetching.delete(guildId);
    });
}

// `quiet`: a background refresh keeps the last info on failure and shows no
// toast. Each alt-tab used to show the same server error again.
// `force`: start a new read even if one is already in flight. A server event
// must never join a request that started before the change it announces.
let infoRequestSeq = 0;
export function loadGuildInfo(options = {}) {
  const guildId = options.guildId || state.activeGuildId;
  if (!guildId) {
    return Promise.resolve(false);
  }
  const quiet = options.quiet === true;
  const key = options.force
    ? `guild-info:${guildId}:fresh:${++infoRequestSeq}`
    : `guild-info:${guildId}`;
  return dedupe(key, () => loadGuildInfoForGuild(guildId, quiet, options.allowDirty === true));
}

async function loadGuildInfoForGuild(guildId, quiet, allowDirty = false) {
  const token = requestToken(`guild-info:${guildId}`);
  try {
    const info = await apiForGuild(guildId, 'GET', '/overlay/guild');
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return false;
    }
    // A settings field is being edited, or was edited before focus moved away.
    // Applying this response would replace the form and pair the fresh
    // `settingsVersion` with stale typed values, making the next save look
    // current. Keep the old version so the save is refused as a conflict.
    if (
      !allowDirty &&
      state.page === 'guild' &&
      state.tab === 'settings' &&
      (settingsControlActive() || settingsFormDirty())
    ) {
      return false;
    }
    state.info = info;
    state.infoLoaded = true;
    state.infoError = null;
    guildInfoLoadedAt = Date.now();
    writeGuildCache(guildId, 'info', info);
    // The guild member may carry a fresher Discord identity: follow it into the
    // device profile without blocking the board paint. The module itself
    // checks that this account is the remembered follower.
    syncProfileFromDiscord(info.member).catch(() => {});
    // The role may have changed while the guild was cached (a demotion, a
    // revoked permission): fix the tab before painting it.
    if (state.tab === 'admin' && !canAdmin()) {
      state.tab = 'board';
      renderView();
    } else if (state.tab === 'board' && state.page === 'guild' && !can('viewBoard')) {
      renderView();
    } else if (state.page === 'settings' || state.tab === 'settings') {
      renderView();
    }
    renderHeader();
    return true;
  } catch (error) {
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return false;
    }
    state.infoError = error.message;
    if (error.code === 'unauthorized') {
      invalidateGuildCache(guildId);
    }
    if (!quiet) {
      notify(error.message, true);
    }
    renderHeader();
    if (state.tab === 'admin' || state.tab === 'settings' || state.page === 'settings') {
      renderView();
    }
    return false;
  }
}

// Reads everything a guild view needs in parallel. Each loader paints its own
// zone as soon as it lands; `switchingGuild` only stops once all of them are
// settled, so the header is not painted twice for the same switch.
export async function hydrateGuild(guildId, options = {}) {
  if (!guildId) {
    return;
  }
  const quiet = options.quiet === true;
  await Promise.allSettled([
    loadGuildInfo({ guildId, quiet }),
    loadItems({ guildId, quiet }),
    loadQueue({ guildId, quiet }),
    loadBindings(guildId),
  ]);
  if (guildId !== state.activeGuildId) {
    return;
  }
  if (state.tab === 'admin' && canAdmin()) {
    await loadAdmin({ quiet });
  }
  if (guildId !== state.activeGuildId) {
    return;
  }
  state.switchingGuild = false;
  renderHeader();
}

export async function loadGuild(options = {}) {
  const guildId = state.activeGuildId;
  if (!guildId) {
    return;
  }
  await hydrateGuild(guildId, options);
}

// `options` may be a click event (toolbar button): only `quiet === true` and
// `guildId` count.
export function loadItems(options = {}) {
  const guildId = options.guildId || state.activeGuildId;
  if (!guildId) {
    return Promise.resolve();
  }
  const quiet = options.quiet === true;
  return dedupe(`items:${guildId}`, () => loadItemsForGuild(guildId, quiet));
}

async function loadItemsForGuild(guildId, quiet) {
  const token = requestToken(`items:${guildId}`);
  try {
    const body = await apiForGuild(guildId, 'GET', '/overlay/media-board/items?limit=200');
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return;
    }
    state.items = body.items || [];
    state.itemsLoaded = true;
    state.itemsError = null;
    state.selection = pruneSelection(state.selection, state.items.map((item) => item.id));
    writeGuildCache(guildId, 'items', state.items);
    scheduleItemsRefresh();
  } catch (error) {
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return;
    }
    state.itemsLoaded = true;
    if (error.code === 'forbidden') {
      state.items = [];
      state.itemsError = null;
      writeGuildCache(guildId, 'items', []);
    } else {
      // A transient failure (rate limit, server restarting) keeps the cards
      // already shown instead of emptying the board.
      state.itemsError = error.message;
      if (!quiet) {
        notify(error.message, true);
      }
    }
  }
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

export function loadQueue(options = {}) {
  const guildId = options.guildId || state.activeGuildId;
  if (!guildId) {
    return Promise.resolve();
  }
  if (document.hidden) {
    state.queueStale = true;
    return Promise.resolve();
  }
  const quiet = options.quiet === true;
  const render = options.render !== false;
  if (options.fresh === true) {
    // A pause/resume changed `startedAt` server-side: this read must supersede
    // one already in flight, otherwise its older snapshot could paint after the
    // resume. `loadQueueForGuild` takes a new token, so the older read is
    // dropped by `isLatest`.
    return loadQueueForGuild(guildId, quiet, render);
  }
  return dedupe(`queue:${guildId}`, () => loadQueueForGuild(guildId, quiet, render));
}

async function loadQueueForGuild(guildId, quiet, render = true) {
  const token = requestToken(`queue:${guildId}`);
  try {
    const body = await apiForGuild(guildId, 'GET', '/overlay/queue');
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return;
    }
    state.queue = { playing: body.playing || null, pending: body.pending || [], at: Date.now() };
    state.queueLoaded = true;
    state.queueError = null;
    state.queueStale = false;
    writeGuildCache(guildId, 'queue', { playing: state.queue.playing, pending: state.queue.pending });
    // Board cards listen to this event to update their playback badges without
    // re-rendering the view (the detail panel may hold a playing video).
    document.dispatchEvent(new CustomEvent('livechat:queue'));
  } catch (error) {
    if (!isLatest(token) || guildId !== state.activeGuildId) {
      return;
    }
    state.queueLoaded = true;
    if (!render) {
      // Background anchor reconciliation: keep the view (and its running
      // animation) untouched instead of repainting the same job.
      return;
    }
    state.queueError = error.message;
    if (!quiet) {
      notify(error.message, true);
    }
    renderHeader();
    renderOnAir();
    if (state.tab === 'queue' && state.page === 'guild') {
      renderView();
    }
    return;
  }
  // The history only matters on its own tab: reading it on every background
  // queue refresh would spend a request per played media.
  if (render && state.tab === 'queue' && state.page === 'guild' && can('viewQueue')) {
    try {
      const body = await apiForGuild(guildId, 'GET', '/overlay/queue/history?limit=30');
      if (isLatest(token) && guildId === state.activeGuildId) {
        state.queueHistory = body.items || [];
        writeGuildCache(guildId, 'history', state.queueHistory);
      }
    } catch {
      // A failed history read keeps the list already shown.
    }
  }
  if (render) {
    renderHeader();
    renderOnAir();
    if (state.tab === 'queue' && state.page === 'guild') {
      renderView();
    }
  }
  scheduleQueueEndRefresh();
}

async function loadBindings(guildId) {
  return dedupe(`bindings:${guildId}`, async () => {
    try {
      const result = await invoke('media_board_get_bindings');
      if (guildId !== state.activeGuildId) {
        return;
      }
      state.bindings = (result && result.bindings) || {};
      writeGuildCache(guildId, 'bindings', state.bindings);
    } catch {
      // A failed read keeps the bindings already shown.
    }
  });
}

// A failed read keeps the list already shown; `quiet` also hides the toast.
async function readList(path, quiet, guildId) {
  try {
    const body = await apiForGuild(guildId, 'GET', path);
    if (guildId !== state.activeGuildId) {
      return null;
    }
    return body.items || [];
  } catch (error) {
    if (guildId !== state.activeGuildId) {
      return null;
    }
    if (!quiet) {
      notify(error.message, true);
    }
    return null;
  }
}

export function loadAdmin(options = {}) {
  const guildId = state.activeGuildId;
  if (!guildId) {
    return Promise.resolve();
  }
  const quiet = options.quiet === true;
  const adminTab = state.adminTab;
  // `force` bypasses the in-flight dedupe: an `admin.changed` hint must trigger
  // a read that started after the mutation, not join one that may predate it.
  const key = options.force
    ? `admin:${guildId}:${adminTab}:fresh:${++adminForcedSeq}`
    : `admin:${guildId}:${adminTab}`;
  return dedupe(key, () => {
    const token = ++adminReadToken;
    return loadAdminForGuild(guildId, quiet, token);
  });
}

async function loadAdminForGuild(guildId, quiet, token) {
  const apply = (assign) => (items) => {
    if (items && token === adminReadToken && guildId === state.activeGuildId) {
      assign(items);
    }
  };
  // Counts of the sub-tabs are known before opening them: members, devices and
  // active codes are read together.
  const jobs = [];
  if (can('manageMembers')) {
    jobs.push(readList('/overlay/admin/members', quiet, guildId).then(apply((items) => {
      state.members = items;
    })));
    jobs.push(readList('/overlay/admin/devices', quiet, guildId).then(apply((items) => {
      state.devices = items;
    })));
  }
  if (can('manageInvites')) {
    jobs.push(readList('/overlay/admin/invites', quiet, guildId).then(apply((items) => {
      state.invites = items;
    })));
    jobs.push(readList('/overlay/admin/pairing-codes', quiet, guildId).then(apply((items) => {
      state.pairingCodes = items;
    })));
  }
  if (state.adminTab === 'bans' && can('manageMembers')) {
    jobs.push(readList('/overlay/admin/bans', quiet, guildId).then(apply((items) => {
      state.bans = items;
    })));
  }
  if (state.adminTab === 'audit' && can('manageSettings')) {
    jobs.push(readList('/overlay/admin/audit?limit=50', quiet, guildId).then(apply((items) => {
      state.auditEntries = items;
    })));
  }
  await Promise.allSettled(jobs);
  if (token !== adminReadToken || guildId !== state.activeGuildId) {
    return;
  }
  if (state.tab === 'admin' && state.page === 'guild') {
    renderView();
  }
}
