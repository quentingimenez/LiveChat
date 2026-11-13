// Background uploads: link submissions, chunked file uploads and their
// provisional cards on the board.
//
// An upload never blocks the dialog: it is added to `state.uploads`, rendered
// as a provisional card, and handled in the background. At most
// MAX_ACTIVE_UPLOADS requests run at once; once the server accepts a media the
// entry switches to `converting`, which frees its slot.

import { openContextMenu } from '../core/contextmenu.js';
import { h, icon, registerIcons } from '../core/dom.js';
import { invoke, listen } from '../core/ipc.js';
import { ERROR_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import {
  chunkRanges,
  defaultTitle,
  ingestionErrorMessage,
  isUploadTooLarge,
  uploadErrorMessage,
} from '../core/upload-helpers.js';
import { requestItemsReload } from '../data.js';

const CHUNK_SIZE = 4 * 1024 * 1024;
const MAX_ACTIVE_UPLOADS = 3;
const MAX_PENDING_INGESTIONS = 50;
const RESYNC_INTERVAL_MS = 15000;
let nextUploadId = 1;
// Results received before their 202 response registered the ingestionId.
const pendingIngestions = new Map();
// Guilds whose ingestion list is currently being read.
const resyncingGuilds = new Set();
let resyncTimer = null;

// No link glyph exists in core/dom.js: add one from this feature module.
registerIcons({
  link: 'M10.6 13.4a4 4 0 005.6 0l2.4-2.4a4 4 0 00-5.7-5.7l-1.3 1.3M13.4 10.6a4 4 0 00-5.6 0l-2.4 2.4a4 4 0 005.7 5.7l1.3-1.3',
});

function uploadsChanged(entry) {
  syncResyncFallback();
  document.dispatchEvent(new CustomEvent('livechat:uploads', { detail: entry ? { id: entry.id } : null }));
}

function findUpload(id) {
  return state.uploads.find((entry) => entry.id === id) || null;
}

function findUploadByIngestion(ingestionId) {
  return state.uploads.find((entry) => entry.ingestionId === ingestionId) || null;
}

function isLive(entry) {
  return state.uploads.includes(entry);
}

function convertingEntriesForGuild(guildId) {
  return state.uploads.filter(
    (entry) => entry.guildId === guildId && entry.status === 'converting' && !entry.cancelled,
  );
}

function rejectionMessage(error) {
  if (typeof error === 'string') {
    return error;
  }
  return (error && error.message) || 'L’envoi a échoué.';
}

function uploadTitle(entry) {
  if (entry.title) {
    return entry.title;
  }
  if (entry.kind === 'link') {
    try {
      return new URL(entry.url).hostname.replace(/^www\./, '');
    } catch {
      return entry.url || 'Lien';
    }
  }
  return defaultTitle(entry.label) || 'Fichier';
}

function newId() {
  const id = `upload-${Date.now()}-${nextUploadId}`;
  nextUploadId += 1;
  return id;
}

export function uploadsForGuild(guildId) {
  return state.uploads.filter((entry) => entry.guildId === guildId);
}

export function startLinkUpload({ guildId, url, title = '', message = '' }) {
  if (!guildId || !url) {
    return null;
  }
  const entry = {
    id: newId(),
    guildId,
    kind: 'link',
    label: url,
    url,
    title: String(title || '').trim(),
    message: message || '',
    status: 'waiting',
    error: null,
    uploadId: null,
    ingestionId: null,
    retryable: true,
    cancelled: false,
  };
  state.uploads.push(entry);
  uploadsChanged(entry);
  pump();
  return entry;
}

export function startFileUpload({ guildId, file, title = '', message = '' }) {
  if (!guildId || !file) {
    return null;
  }
  const label = file.name || 'Fichier';
  const tooLarge = isUploadTooLarge(file.size);
  const entry = {
    id: newId(),
    guildId,
    kind: 'file',
    label,
    file,
    title: String(title || '').trim() || defaultTitle(label),
    message: message || '',
    status: tooLarge ? 'failed' : 'waiting',
    error: tooLarge ? 'Fichier trop volumineux : 100 Mo maximum.' : null,
    uploadId: null,
    ingestionId: null,
    // An oversized file cannot be retried without choosing another file.
    retryable: !tooLarge,
    cancelled: false,
  };
  state.uploads.push(entry);
  uploadsChanged(entry);
  pump();
  return entry;
}

function removeEntry(entry) {
  const index = state.uploads.indexOf(entry);
  if (index !== -1) {
    state.uploads.splice(index, 1);
  }
  uploadsChanged(entry);
}

function fail(entry, message) {
  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  entry.status = 'failed';
  entry.error = message || 'L’envoi a échoué.';
  entry.uploadId = null;
  entry.ingestionId = null;
  uploadsChanged(entry);
}

function failIngestion(entry, error) {
  fail(entry, ingestionErrorMessage(error, ERROR_LABELS));
}

function rememberPendingIngestion(result) {
  pendingIngestions.set(result.ingestionId, result);
  while (pendingIngestions.size > MAX_PENDING_INGESTIONS) {
    const oldest = pendingIngestions.keys().next().value;
    pendingIngestions.delete(oldest);
  }
}

function takePendingIngestion(ingestionId) {
  const result = pendingIngestions.get(ingestionId);
  if (result) {
    pendingIngestions.delete(ingestionId);
  }
  return result || null;
}

function markConverting(entry, ingestionId) {
  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  entry.status = 'converting';
  entry.ingestionId = ingestionId;
  entry.uploadId = null;
  entry.error = null;
  uploadsChanged(entry);
  // A result may have arrived before the 202 response was handled.
  const pending = takePendingIngestion(ingestionId);
  if (pending) {
    applyIngestionResult(entry, pending);
  }
}

function finishIngestion(entry, result) {
  if (!isLive(entry) || entry.cancelled || entry.status !== 'converting') {
    return;
  }
  const title = uploadTitle(entry);
  if (result.created === false) {
    removeEntry(entry);
    notify(`« ${title} » est déjà sur le board.`);
    return;
  }
  // Another guild's board is not on screen: nothing to reload.
  if (entry.guildId !== state.activeGuildId) {
    removeEntry(entry);
    notify(`« ${title} » ajouté au board.`);
    return;
  }
  // Keep the provisional card until the real one is in the grid.
  requestItemsReload().then(() => {
    if (!isLive(entry)) {
      return;
    }
    removeEntry(entry);
    notify(`« ${title} » ajouté au board.`);
  });
}

function applyIngestionResult(entry, result) {
  if (!isLive(entry) || entry.cancelled || entry.status !== 'converting') {
    return;
  }
  if (!result || result.ok !== true) {
    failIngestion(entry, result && result.error);
    return;
  }
  finishIngestion(entry, result);
}

function acceptIngestion(entry, response) {
  const ingestionId = response && response.body && response.body.ingestionId;
  if (typeof ingestionId !== 'string' || !ingestionId) {
    fail(entry, 'Réponse inattendue du serveur.');
    return;
  }
  markConverting(entry, ingestionId);
}

function handleMediaIngestion(result) {
  if (!result || typeof result.ingestionId !== 'string' || !result.ingestionId) {
    return;
  }
  const entry = findUploadByIngestion(result.ingestionId);
  if (entry) {
    applyIngestionResult(entry, result);
    return;
  }
  rememberPendingIngestion(result);
}

function guildIsConnected(guildId) {
  const guild = state.guilds.find(
    (candidate) => candidate && candidate.membership && candidate.membership.guildId === guildId,
  );
  return Boolean(guild && guild.connection && guild.connection.status === 'connected');
}

function guildsNeedingFallback() {
  const guilds = new Set();
  for (const entry of state.uploads) {
    if (entry.status === 'converting' && !entry.cancelled && !guildIsConnected(entry.guildId)) {
      guilds.add(entry.guildId);
    }
  }
  return [...guilds];
}

// While a guild has converting entries but no live WebSocket, poll its
// ingestion list. The timer stops as soon as no guild needs it.
function syncResyncFallback() {
  const guilds = guildsNeedingFallback();
  if (guilds.length === 0) {
    if (resyncTimer !== null) {
      clearInterval(resyncTimer);
      resyncTimer = null;
    }
    return;
  }
  if (resyncTimer !== null) {
    return;
  }
  resyncTimer = setInterval(() => {
    const targets = guildsNeedingFallback();
    if (targets.length === 0) {
      clearInterval(resyncTimer);
      resyncTimer = null;
      return;
    }
    for (const guildId of targets) {
      resyncGuild(guildId);
    }
  }, RESYNC_INTERVAL_MS);
}

async function resyncGuild(guildId) {
  const entries = convertingEntriesForGuild(guildId);
  if (entries.length === 0) {
    syncResyncFallback();
    return;
  }
  if (resyncingGuilds.has(guildId)) {
    return;
  }
  resyncingGuilds.add(guildId);
  try {
    const response = await invoke('guild_api', {
      payload: {
        guildId,
        method: 'GET',
        path: '/overlay/media-board/ingestions',
      },
    });
    if (!response || !response.ok) {
      return;
    }
    const items = response.body && Array.isArray(response.body.items) ? response.body.items : [];
    const byId = new Map();
    for (const item of items) {
      if (item && typeof item.ingestionId === 'string') {
        byId.set(item.ingestionId, item);
      }
    }
    for (const entry of entries) {
      if (!isLive(entry) || entry.status !== 'converting') {
        continue;
      }
      const item = byId.get(entry.ingestionId);
      if (!item) {
        fail(entry, 'Le serveur a redémarré pendant la conversion.');
      } else if (item.status === 'ready') {
        applyIngestionResult(entry, {
          ingestionId: entry.ingestionId,
          ok: true,
          created: item.created,
          itemId: item.itemId,
        });
      } else if (item.status === 'failed') {
        applyIngestionResult(entry, {
          ingestionId: entry.ingestionId,
          ok: false,
          error: item.error,
        });
      }
    }
  } catch {
    // Transient read failure: the WebSocket result or the next poll finishes it.
  } finally {
    resyncingGuilds.delete(guildId);
    syncResyncFallback();
  }
}

export function initUploadIngestion() {
  listen('overlay://media-ingestion', handleMediaIngestion);
  document.addEventListener('livechat:guild-connected', (event) => {
    const guildId = event && event.detail ? event.detail.guildId : null;
    if (!guildId) {
      return;
    }
    syncResyncFallback();
    resyncGuild(guildId);
  });
  // A guild may also disconnect while an ingestion is converting: start the
  // fallback poll in that case.
  document.addEventListener('livechat:guilds-updated', syncResyncFallback);
}

function cancelRemote(uploadId) {
  return invoke('media_upload_cancel', { payload: { uploadId } }).catch(() => false);
}

// Starts waiting uploads while fewer than MAX_ACTIVE_UPLOADS are in flight.
function pump() {
  let slots = MAX_ACTIVE_UPLOADS - state.uploads.filter((entry) => entry.status === 'sending' && !entry.cancelled).length;
  for (const entry of state.uploads) {
    if (slots <= 0) {
      break;
    }
    if (entry.status !== 'waiting' || entry.cancelled) {
      continue;
    }
    entry.status = 'sending';
    entry.error = null;
    slots -= 1;
    uploadsChanged(entry);
    runUpload(entry);
  }
}

async function runUpload(entry) {
  try {
    if (entry.kind === 'link') {
      await sendLink(entry);
    } else {
      await sendFile(entry);
    }
  } catch (error) {
    fail(entry, rejectionMessage(error));
  } finally {
    pump();
  }
}

async function sendLink(entry) {
  let response;
  try {
    response = await invoke('guild_api', {
      payload: {
        guildId: entry.guildId,
        method: 'POST',
        path: '/overlay/media-board/items',
        body: { url: entry.url, title: entry.title, message: entry.message },
      },
    });
  } catch (error) {
    fail(entry, rejectionMessage(error));
    return;
  }
  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  if (!response || !response.ok) {
    fail(entry, uploadErrorMessage(response && response.status, response && response.body, ERROR_LABELS));
    return;
  }
  if (response.status !== 202) {
    fail(entry, uploadErrorMessage(response.status, response.body, ERROR_LABELS));
    return;
  }
  acceptIngestion(entry, response);
}

async function sendFile(entry) {
  let started;
  try {
    started = await invoke('media_upload_start', {
      payload: {
        guildId: entry.guildId,
        filename: entry.label,
        title: entry.title,
        message: entry.message,
        contentType: entry.file.type || null,
      },
    });
  } catch (error) {
    fail(entry, rejectionMessage(error));
    return;
  }
  const uploadId = started && started.uploadId;
  if (!isLive(entry) || entry.cancelled) {
    if (uploadId) {
      cancelRemote(uploadId);
    }
    return;
  }
  if (!uploadId) {
    fail(entry, 'Le serveur n’a pas accepté l’envoi.');
    return;
  }
  entry.uploadId = uploadId;

  for (const [start, end] of chunkRanges(entry.file.size, CHUNK_SIZE)) {
    if (!isLive(entry) || entry.cancelled) {
      return;
    }
    let bytes;
    try {
      bytes = new Uint8Array(await entry.file.slice(start, end).arrayBuffer());
    } catch (error) {
      if (!isLive(entry) || entry.cancelled) {
        return;
      }
      await cancelRemote(uploadId);
      fail(entry, rejectionMessage(error));
      return;
    }
    if (!isLive(entry) || entry.cancelled) {
      return;
    }
    try {
      await invoke('media_upload_chunk', bytes, { headers: { 'x-upload-id': uploadId } });
    } catch (error) {
      if (!isLive(entry) || entry.cancelled) {
        return;
      }
      // The server already answered (e.g. file too large): stop sending and
      // collect its final response.
      if (rejectionMessage(error).includes('upload_closed')) {
        break;
      }
      await cancelRemote(uploadId);
      fail(entry, rejectionMessage(error));
      return;
    }
  }

  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  let result;
  try {
    result = await invoke('media_upload_finish', { payload: { uploadId } });
  } catch (error) {
    if (!isLive(entry) || entry.cancelled) {
      return;
    }
    await cancelRemote(uploadId);
    fail(entry, rejectionMessage(error));
    return;
  }
  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  if (!result || !result.ok) {
    await cancelRemote(uploadId);
    fail(entry, uploadErrorMessage(result && result.status, result && result.body, ERROR_LABELS));
    return;
  }
  if (result.status !== 202) {
    fail(entry, uploadErrorMessage(result.status, result.body, ERROR_LABELS));
    return;
  }
  acceptIngestion(entry, result);
}

export function retryUpload(id) {
  const entry = findUpload(id);
  if (!entry || entry.status !== 'failed' || entry.retryable === false) {
    return;
  }
  entry.status = 'waiting';
  entry.error = null;
  entry.uploadId = null;
  entry.ingestionId = null;
  entry.cancelled = false;
  uploadsChanged(entry);
  pump();
}

// Cancels an in-flight upload: the server is told, the entry disappears right
// away and the next waiting upload takes its slot.
export async function cancelUpload(id) {
  const entry = findUpload(id);
  if (!entry) {
    return;
  }
  entry.cancelled = true;
  const uploadId = entry.uploadId;
  const cancellation = uploadId ? cancelRemote(uploadId) : Promise.resolve(false);
  removeEntry(entry);
  pump();
  await cancellation;
}

export function removeUpload(id) {
  const entry = findUpload(id);
  if (!entry) {
    return;
  }
  entry.cancelled = true;
  removeEntry(entry);
  pump();
}

function uploadKind(entry) {
  if (entry.kind === 'link') {
    return 'link';
  }
  const type = (entry.file && entry.file.type) || '';
  if (type.startsWith('video/')) {
    return 'video';
  }
  if (type.startsWith('image/')) {
    return 'image';
  }
  if (type.startsWith('audio/')) {
    return 'audio';
  }
  return 'copy';
}

function uploadStateNode(entry) {
  if (entry.status === 'failed') {
    return h(
      'div',
      { class: 'upload-state is-failed', 'aria-live': 'polite' },
      h('p', { class: 'upload-error', title: entry.error }, entry.error || 'L’envoi a échoué.'),
      h(
        'div',
        { class: 'upload-actions' },
        entry.retryable === false
          ? null
          : h('button', { class: 'btn', type: 'button', onClick: () => retryUpload(entry.id) }, 'Réessayer'),
        h('button', { class: 'btn is-quiet', type: 'button', onClick: () => removeUpload(entry.id) }, 'Retirer'),
      ),
    );
  }
  let label = 'En attente…';
  if (entry.status === 'sending') {
    label = entry.kind === 'link' ? 'Téléchargement…' : 'Envoi…';
  } else if (entry.status === 'converting') {
    label = 'Conversion…';
  }
  return h('div', { class: 'upload-state', 'aria-live': 'polite' }, label);
}

// Same size and structure as a normal card (thumbnail + title) so the grid
// does not move when the provisional card is replaced by the real one.
export function renderUploadCard(entry) {
  const title = uploadTitle(entry);
  return h(
    'div',
    { class: 'card is-upload', 'data-upload-id': entry.id, 'data-guild-id': entry.guildId },
    h(
      'div',
      { class: 'card-frame' },
      h(
        'div',
        { class: `thumb is-upload${entry.status === 'failed' ? ' is-failed' : ''}` },
        h('span', { class: 'glyph' }, icon(uploadKind(entry))),
        uploadStateNode(entry),
      ),
    ),
    h('p', { class: 'card-title', title }, title),
  );
}

export function openUploadMenu(event, entry) {
  openContextMenu(event, [
    { heading: uploadTitle(entry) },
    entry.status === 'failed' && entry.retryable !== false
      ? { label: 'Réessayer', icon: 'refresh', onSelect: () => retryUpload(entry.id) }
      : null,
    entry.status === 'waiting' || entry.status === 'sending'
      ? { label: 'Annuler l’envoi', icon: 'close', onSelect: () => { cancelUpload(entry.id); } }
      : null,
    entry.status === 'failed' ? { label: 'Retirer', icon: 'remove', danger: true, onSelect: () => removeUpload(entry.id) } : null,
  ]);
}
