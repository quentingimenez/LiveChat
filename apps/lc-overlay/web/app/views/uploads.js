// Background uploads: link submissions, chunked file uploads and their
// provisional cards on the board.
//
// An upload never blocks the dialog: it is added to `state.uploads`, rendered
// as a provisional card, and handled in the background. At most
// MAX_ACTIVE_UPLOADS run at once; the others wait.

import { openContextMenu } from '../core/contextmenu.js';
import { h, icon, registerIcons } from '../core/dom.js';
import { invoke } from '../core/ipc.js';
import { ERROR_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { chunkRanges, defaultTitle, uploadErrorMessage } from '../core/upload-helpers.js';
import { loadItems } from '../data.js';

const CHUNK_SIZE = 4 * 1024 * 1024;
const MAX_ACTIVE_UPLOADS = 3;
let nextUploadId = 1;

// No link glyph exists in core/dom.js: add one from this feature module.
registerIcons({
  link: 'M10.6 13.4a4 4 0 005.6 0l2.4-2.4a4 4 0 00-5.7-5.7l-1.3 1.3M13.4 10.6a4 4 0 00-5.6 0l-2.4 2.4a4 4 0 005.7 5.7l1.3-1.3',
});

function uploadsChanged(entry) {
  document.dispatchEvent(new CustomEvent('livechat:uploads', { detail: entry ? { id: entry.id } : null }));
}

function findUpload(id) {
  return state.uploads.find((entry) => entry.id === id) || null;
}

function isLive(entry) {
  return state.uploads.includes(entry);
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
  const entry = {
    id: newId(),
    guildId,
    kind: 'file',
    label,
    file,
    title: String(title || '').trim() || defaultTitle(label),
    message: message || '',
    status: 'waiting',
    error: null,
    uploadId: null,
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
  uploadsChanged(entry);
}

function finishSuccess(entry, body) {
  if (!isLive(entry) || entry.cancelled) {
    return;
  }
  const title = uploadTitle(entry);
  removeEntry(entry);
  notify(body && body.created === false ? `« ${title} » est déjà sur le board.` : `« ${title} » ajouté au board.`);
  if (entry.guildId === state.activeGuildId) {
    loadItems();
  }
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
  finishSuccess(entry, response.body);
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
  finishSuccess(entry, result.body);
}

export function retryUpload(id) {
  const entry = findUpload(id);
  if (!entry || entry.status !== 'failed') {
    return;
  }
  entry.status = 'waiting';
  entry.error = null;
  entry.uploadId = null;
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
        h('button', { class: 'btn', type: 'button', onClick: () => retryUpload(entry.id) }, 'Réessayer'),
        h('button', { class: 'btn is-quiet', type: 'button', onClick: () => removeUpload(entry.id) }, 'Retirer'),
      ),
    );
  }
  return h('div', { class: 'upload-state', 'aria-live': 'polite' }, entry.status === 'sending' ? 'Téléchargement…' : 'En attente…');
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
    entry.status === 'failed' ? { label: 'Réessayer', icon: 'refresh', onSelect: () => retryUpload(entry.id) } : null,
    entry.status === 'waiting' || entry.status === 'sending'
      ? { label: 'Annuler l’envoi', icon: 'close', onSelect: () => { cancelUpload(entry.id); } }
      : null,
    entry.status === 'failed' ? { label: 'Retirer', icon: 'remove', danger: true, onSelect: () => removeUpload(entry.id) } : null,
  ]);
}
