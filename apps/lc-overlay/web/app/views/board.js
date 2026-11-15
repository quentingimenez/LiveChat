// Media board: grid, cards, detail panel, play, shortcuts, edit and delete.

import { api, run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { acceleratorFromEvent, acceleratorLabel } from '../core/accelerators.js';
import { h, icon } from '../core/dom.js';
import { formatDuration } from '../core/format.js';
import { cardRatio, cardStyle } from '../core/masonry.js';
import { invoke } from '../core/ipc.js';
import { activeGuild, can, describeError, KIND_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { loadItems, refreshQueueAutomatically } from '../data.js';
import { openAddDialog } from './add-media.js';
import { openUploadMenu, renderUploadCard, startFileUpload, uploadsForGuild } from './uploads.js';
import { renderView } from './view.js';

export function itemTitle(item) {
  if (item.title) {
    return item.title;
  }
  try {
    return new URL(item.media.sourceUrl).hostname.replace(/^www\./, '');
  } catch {
    return 'Sans titre';
  }
}

function openCardMenu(event, item) {
  const sourceUrl = item.media && item.media.sourceUrl;
  const copyable = typeof sourceUrl === 'string' && /^https?:\/\//i.test(sourceUrl);
  openContextMenu(event, [
    { heading: itemTitle(item) },
    can('play') ? { label: 'Jouer sur les overlays', icon: 'play', onSelect: () => playItem(item) } : null,
    {
      label: state.selectedId === item.id ? 'Masquer les détails' : 'Afficher les détails',
      onSelect: () => selectItem(item.id),
    },
    item.canEdit ? { label: 'Modifier…', onSelect: () => openEditDialog(item) } : null,
    can('play')
      ? {
          label: shortcutFor(item.id) ? 'Changer le raccourci' : 'Définir un raccourci',
          onSelect: () => {
            state.selectedId = item.id;
            state.capturing = false;
            startCapture(item);
          },
        }
      : null,
    copyable ? { label: 'Copier le lien d’origine', icon: 'copy', onSelect: () => copyText(sourceUrl, 'Lien copié.') } : null,
    item.canEdit ? 'separator' : null,
    item.canEdit ? { label: 'Supprimer…', icon: 'remove', danger: true, onSelect: () => openDeleteDialog(item) } : null,
  ]);
}

function visibleItems() {
  const needle = state.search.trim().toLowerCase();
  return state.items.filter((item) => {
    if (state.kind !== 'all' && item.media.kind !== state.kind) {
      return false;
    }
    if (!needle) {
      return true;
    }
    return [item.title, item.message, item.createdByName, item.media.sourceUrl]
      .filter(Boolean)
      .some((value) => value.toLowerCase().includes(needle));
  });
}

// --- card cache ---
// Card nodes live across renders, keyed by item id. Rebuilding them on every
// render (mute, filter, board reload) reloaded each thumbnail and made the
// grid flash. `entry.item` always points at the latest copy of the item, so
// the card's handlers never act on stale data.
const cards = new Map(); // itemId -> { node, key, item, loadedUrl }
// The preview of the selected item, kept across renders of the view:
// rebuilding it reloaded the video and showed the loading spinner again.
let previewCache = null; // { itemId, path, node }

// Path of a signed media URL, without its query: the same asset keeps the
// same path while its signature changes every 15 minutes.
function mediaPath(url) {
  if (typeof url !== 'string' || !url) {
    return null;
  }
  try {
    const parsed = new URL(url);
    return `${parsed.origin}${parsed.pathname}`;
  } catch {
    return url;
  }
}

// Expiry of a signed URL (`exp`, Unix seconds), in milliseconds.
function urlExpiresAt(url) {
  try {
    const exp = Number(new URL(url).searchParams.get('exp'));
    return Number.isFinite(exp) && exp > 0 ? exp * 1000 : 0;
  } catch {
    return 0;
  }
}

function cardKey(item) {
  return JSON.stringify([
    itemTitle(item),
    item.media.kind,
    item.media.durationSec ?? null,
    item.media.width ?? null,
    item.media.height ?? null,
    mediaPath(item.mediaUrl),
    can('play'),
  ]);
}

// Thumbnails load their media only when they come close to the viewport.
const thumbObserver = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) {
        continue;
      }
      thumbObserver.unobserve(entry.target);
      const { mediaUrl, kind, itemId } = entry.target.dataset;
      if (!mediaUrl) {
        continue;
      }
      const media =
        kind === 'video'
          ? h('video', { src: `${mediaUrl}#t=0.1`, preload: 'metadata', muted: true, playsinline: true, tabindex: '-1' })
          : h('img', { src: mediaUrl, alt: '', loading: 'lazy' });
      // The fallback glyph goes away as soon as the thumbnail can be shown.
      const glyph = entry.target.querySelector('.glyph');
      media.addEventListener('error', () => media.remove());
      media.addEventListener(kind === 'video' ? 'loadeddata' : 'load', () => {
        if (glyph) {
          glyph.remove();
        }
        // The detail panel reuses this URL: the browser already has its bytes.
        const card = cards.get(itemId);
        if (card) {
          card.loadedUrl = mediaUrl;
        }
      });
      entry.target.prepend(media);
    }
  },
  { rootMargin: '200px' },
);

// --- playback badges ---
// `state.triggers` maps jobId -> itemId. `state.triggerHints` keeps the
// outcome returned by the click until a newer read of the queue decides.
function queueBadgeFor(jobId) {
  const { playing, pending } = state.queue;
  if (playing && playing.id === jobId) {
    return { air: true, label: 'À l’écran' };
  }
  const index = (pending || []).findIndex((job) => job.id === jobId);
  if (index !== -1) {
    return { air: false, label: `En file · ${index + 1}` };
  }
  return null;
}

// A job missing from a queue read newer than the click is over: forget it so
// its badge disappears for good.
function pruneTriggers() {
  const queueAt = state.queue.at || 0;
  for (const [jobId, hint] of state.triggerHints) {
    if (queueAt <= hint.at || queueBadgeFor(jobId)) {
      continue;
    }
    state.triggerHints.delete(jobId);
    state.triggers.delete(jobId);
  }
}

function triggerBadgeFor(itemId) {
  pruneTriggers();
  let queued = null;
  for (const [jobId, mappedItemId] of state.triggers) {
    if (mappedItemId !== itemId) {
      continue;
    }
    const inQueue = queueBadgeFor(jobId);
    if (!inQueue) {
      continue;
    }
    if (inQueue.air) {
      return inQueue;
    }
    queued = queued || inQueue;
  }
  if (queued) {
    return queued;
  }
  for (const [jobId, mappedItemId] of state.triggers) {
    if (mappedItemId !== itemId) {
      continue;
    }
    const hint = state.triggerHints.get(jobId);
    if (!hint) {
      continue;
    }
    if (hint.outcome === 'playing') {
      return { air: true, label: 'À l’écran' };
    }
    if (hint.outcome === 'queued') {
      queued = { air: false, label: hint.position ? `En file · ${hint.position}` : 'En file' };
    }
  }
  return queued;
}

// Updates badges in place: re-rendering the whole view would reload the video
// in the detail panel.
function refreshTriggerBadges() {
  if (state.tab !== 'board' || state.page !== 'guild') {
    return;
  }
  pruneTriggers();
  for (const card of document.querySelectorAll('.card[data-item-id]')) {
    syncBadge(card, triggerBadgeFor(card.dataset.itemId));
  }
}

// --- provisional upload cards ---
function activeUploads() {
  return state.activeGuildId ? uploadsForGuild(state.activeGuildId) : [];
}

function uploadCard(entry) {
  const card = renderUploadCard(entry);
  card.addEventListener('contextmenu', (event) => openUploadMenu(event, entry));
  return card;
}

function uploadCards() {
  return activeUploads().map(uploadCard);
}

// Replaces only the provisional cards, keeping normal cards and the detail
// panel (and its video) untouched.
function refreshUploadCards() {
  if (state.tab !== 'board' || state.page !== 'guild' || !state.activeGuildId) {
    return;
  }
  const grid = document.getElementById('grid');
  if (!grid) {
    renderView();
    return;
  }
  const nodes = uploadCards();
  for (const node of grid.querySelectorAll('.card.is-upload')) {
    node.remove();
  }
  if (nodes.length === 0 && grid.children.length === 0) {
    renderView();
    return;
  }
  grid.prepend(...nodes);
}

function buildCard(entry) {
  const item = entry.item;
  const kind = item.media.kind;
  const mediaUrl = typeof item.mediaUrl === 'string' && item.mediaUrl ? item.mediaUrl : null;
  const thumb = h(
    'button',
    {
      class: 'thumb',
      'data-media-url': mediaUrl,
      'data-kind': kind,
      'data-item-id': item.id,
      'aria-label': `Détails de ${itemTitle(item)}`,
      onClick: () => selectItem(entry.item.id),
    },
    h('span', { class: 'glyph' }, icon(kind === 'audio' ? 'audio' : kind === 'image' ? 'image' : 'video')),
    formatDuration(item.media.durationSec) ? h('span', { class: 'duration' }, formatDuration(item.media.durationSec)) : null,
  );
  if (kind !== 'audio' && mediaUrl) {
    thumbObserver.observe(thumb);
  }
  return h(
    'div',
    {
      class: 'card',
      'data-item-id': item.id,
      // Height of the media's own shape (masonry, see core/masonry.js).
      style: cardStyle(cardRatio(item.media)),
      onContextmenu: (event) => openCardMenu(event, entry.item),
    },
    h(
      'div',
      { class: 'card-frame' },
      thumb,
      can('play')
        ? h('button', { class: 'play', 'aria-label': `Jouer ${itemTitle(item)}`, title: 'Jouer sur les overlays', onClick: () => playItem(entry.item) }, icon('play'))
        : null,
    ),
    h('p', { class: 'card-title', title: itemTitle(item) }, itemTitle(item)),
  );
}

// Returns the card of an item, reusing its node while nothing it shows has
// changed. A new signature for the same asset only refreshes the URL a
// thumbnail will load later; a loaded thumbnail keeps its bytes.
function renderCard(item) {
  const key = cardKey(item);
  let entry = cards.get(item.id);
  if (!entry || entry.key !== key) {
    if (entry) {
      entry.node.querySelectorAll('.thumb').forEach((thumb) => thumbObserver.unobserve(thumb));
    }
    entry = { node: null, key, item, loadedUrl: null };
    cards.set(item.id, entry);
    entry.node = buildCard(entry);
  } else {
    entry.item = item;
    const thumb = entry.node.querySelector('.thumb');
    if (thumb && !thumb.querySelector('img, video') && item.mediaUrl) {
      thumb.dataset.mediaUrl = item.mediaUrl;
    }
  }
  entry.node.classList.toggle('is-selected', state.selectedId === item.id);
  syncBadge(entry.node, triggerBadgeFor(item.id));
  return entry.node;
}

// Drops the cards of items that left the board (or of another guild).
function pruneCards() {
  const ids = new Set(state.items.map((item) => item.id));
  for (const [id, entry] of cards) {
    if (!ids.has(id)) {
      entry.node.querySelectorAll('.thumb').forEach((thumb) => thumbObserver.unobserve(thumb));
      cards.delete(id);
    }
  }
}

function syncBadge(card, badge) {
  const frame = card.querySelector('.card-frame');
  if (!frame) {
    return;
  }
  let node = frame.querySelector('.card-badge');
  if (!badge) {
    if (node) {
      node.remove();
    }
    return;
  }
  if (!node) {
    node = h('span', { class: 'card-badge' });
    frame.append(node);
  }
  node.className = `card-badge${badge.air ? ' is-air' : ''}`;
  node.textContent = badge.label;
}

export function renderBoard() {
  if (state.info && !can('viewBoard')) {
    return h('div', { class: 'empty' }, h('h2', null, 'Board réservé aux membres'), h('p', null, "Cette guilde ne montre pas son board aux invités. Tu reçois quand même les médias diffusés."));
  }
  pruneCards();
  const items = visibleItems();
  const uploads = activeUploads();
  const selected = state.items.find((item) => item.id === state.selectedId) || null;
  if (!selected) {
    // A preview no longer shown must not keep its video alive.
    previewCache = null;
  }
  const toolbar = h(
    'div',
    { class: 'toolbar' },
    h('input', {
      class: 'field',
      type: 'search',
      placeholder: 'Rechercher un média',
      value: state.search,
      onInput: (event) => {
        state.search = event.target.value;
        refreshGrid(true);
      },
    }),
    h(
      'div',
      { class: 'chips' },
      Object.entries(KIND_LABELS).map(([id, label]) =>
        h('button', { class: `chip${state.kind === id ? ' is-active' : ''}`, 'data-kind': id, onClick: () => setKind(id) }, label),
      ),
    ),
    h('span', { class: 'spacer' }),
    h('button', { class: 'btn is-quiet', title: 'Recharger le board', 'aria-label': 'Recharger le board', onClick: loadItems }, icon('refresh')),
    can('addMedia') ? h('button', { class: 'btn', onClick: openAddDialog }, icon('plus'), 'Ajouter un média') : null,
  );
  let content;
  if (!state.itemsLoaded && uploads.length === 0) {
    content = h('p', { class: 'meta' }, 'Chargement du board…');
  } else if (state.items.length === 0 && uploads.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Le board est vide'),
      h('p', null, 'Colle le lien d’une vidéo, d’une image ou d’un son pour créer le premier média.'),
      can('addMedia') ? h('button', { class: 'btn', onClick: openAddDialog }, 'Ajouter un média') : null,
    );
  } else if (items.length === 0 && uploads.length === 0) {
    content = h('p', { class: 'meta' }, 'Aucun média ne correspond à cette recherche.');
  } else {
    // Provisional cards always come first, whatever the search and filter.
    content = h('div', { class: 'grid', id: 'grid' }, uploads.map(uploadCard), items.map(renderCard));
  }
  return h(
    'div',
    null,
    toolbar,
    h('div', { class: `board${selected ? ' has-detail' : ''}` }, content, selected ? renderDetail(selected) : null),
  );
}

// Updates the grid in place after a search or a filter: the cards are moved,
// never rebuilt, so their thumbnails stay on screen.
function refreshGrid(fromSearch) {
  const grid = document.getElementById('grid');
  const items = visibleItems();
  const uploads = activeUploads();
  if (!grid || (items.length === 0 && uploads.length === 0)) {
    const input = document.activeElement;
    const position = input && input.selectionStart;
    renderView();
    const field = document.querySelector('.toolbar input');
    if (fromSearch && field) {
      field.focus();
      field.setSelectionRange(position ?? field.value.length, position ?? field.value.length);
    }
    return;
  }
  grid.replaceChildren(...uploads.map(uploadCard), ...items.map(renderCard));
}

function setKind(kind) {
  state.kind = kind;
  for (const chip of document.querySelectorAll('.toolbar .chip')) {
    chip.classList.toggle('is-active', chip.dataset.kind === kind);
  }
  refreshGrid(false);
}

export function selectItem(id) {
  state.selectedId = state.selectedId === id ? null : id;
  state.capturing = false;
  if (!previewCache || previewCache.itemId !== state.selectedId) {
    previewCache = null;
  }
  renderView();
}

export async function playItem(item) {
  const guild = activeGuild();
  if (guild && guild.membership.muted) {
    notify('Guilde en sourdine : rétablis-la pour jouer.', true);
    return;
  }
  let response;
  try {
    response = await invoke('media_board_trigger', { payload: { itemId: item.id, guildId: state.activeGuildId, trigger: 'ui' } });
  } catch (error) {
    notify(typeof error === 'string' ? error : error.message || 'Action impossible.', true);
    return;
  }
  if (!response) {
    return;
  }
  if (!response.ok) {
    notify(describeError(response.reason, response.message), true);
    return;
  }
  if (response.jobId && (response.outcome === 'playing' || response.outcome === 'queued')) {
    // One badge per media: a new trigger replaces the previous job of that
    // item.
    for (const [jobId, mappedItemId] of state.triggers) {
      if (mappedItemId === item.id) {
        state.triggers.delete(jobId);
        state.triggerHints.delete(jobId);
      }
    }
    state.triggers.set(response.jobId, item.id);
    state.triggerHints.set(response.jobId, {
      outcome: response.outcome,
      position: response.position || null,
      at: Date.now(),
    });
  }
  if (response.outcome === 'playing') {
    notify(`« ${itemTitle(item)} » passe à l’écran.`);
  } else if (response.outcome === 'queued') {
    notify(response.position ? `« ${itemTitle(item)} » ajouté à la file (position ${response.position}).` : `« ${itemTitle(item)} » ajouté à la file.`);
  } else {
    notify(`« ${itemTitle(item)} » envoyé aux overlays.`);
  }
  refreshTriggerBadges();
  setTimeout(refreshQueueAutomatically, 400);
}

export function shortcutFor(itemId) {
  const entry = Object.entries(state.bindings).find(([, id]) => id === itemId);
  return entry ? entry[0] : null;
}

// --- detail preview ---
// A loaded thumbnail URL is reused while it stays valid long enough to watch
// the media: the browser already has its first bytes.
const PREVIEW_URL_MARGIN_MS = 10 * 60 * 1000;
// Widest still copied from a thumbnail.
const STILL_MAX_WIDTH = 640;
function previewUrl(item) {
  const card = cards.get(item.id);
  const loaded = card && card.loadedUrl;
  if (loaded && mediaPath(loaded) === mediaPath(item.mediaUrl) && urlExpiresAt(loaded) - Date.now() > PREVIEW_URL_MARGIN_MS) {
    return loaded;
  }
  return item.mediaUrl || null;
}

function ratioOf(width, height) {
  return Number(width) > 0 && Number(height) > 0 ? `${Number(width)} / ${Number(height)}` : null;
}

// Copy of the frame the card already shows, laid over the detail video until
// the video can draw its own first frame.
function cardStill(itemId) {
  const card = cards.get(itemId);
  const video = card && card.node.querySelector('.thumb video');
  if (!video || video.readyState < 2 || !video.videoWidth || !video.videoHeight) {
    return null;
  }
  const scale = Math.min(1, STILL_MAX_WIDTH / video.videoWidth);
  const canvas = h('canvas', {
    class: 'preview-still',
    width: String(Math.round(video.videoWidth * scale)),
    height: String(Math.round(video.videoHeight * scale)),
    'aria-hidden': 'true',
  });
  try {
    canvas.getContext('2d').drawImage(video, 0, 0, canvas.width, canvas.height);
  } catch {
    return null;
  }
  return canvas;
}

function buildPreview(item) {
  const kind = item.media.kind;
  const url = previewUrl(item);
  const node = h('div', { class: `preview${kind === 'audio' ? ' is-audio' : ''}` });
  // The box takes the media's own shape (portrait, square…) instead of 16:10.
  const ratio = ratioOf(item.media.width, item.media.height);
  if (ratio) {
    node.style.setProperty('--ratio', ratio);
  }
  const adoptRatio = (width, height) => {
    if (!ratio && ratioOf(width, height)) {
      node.style.setProperty('--ratio', ratioOf(width, height));
    }
  };
  if (!url) {
    node.append(h('span', { class: 'meta' }, 'Média indisponible.'));
    return node;
  }
  if (kind === 'audio') {
    node.append(h('audio', { src: url, controls: true, preload: 'metadata' }));
    return node;
  }
  if (kind === 'video') {
    const video = h('video', { src: url, controls: true, preload: 'auto', playsinline: true });
    video.addEventListener('loadedmetadata', () => adoptRatio(video.videoWidth, video.videoHeight));
    node.append(video);
    const still = cardStill(item.id);
    if (still) {
      node.append(still);
      const reveal = () => still.remove();
      video.addEventListener('loadeddata', reveal, { once: true });
      video.addEventListener('error', reveal, { once: true });
    }
    return node;
  }
  const image = h('img', { src: url, alt: '' });
  image.addEventListener('load', () => adoptRatio(image.naturalWidth, image.naturalHeight));
  node.append(image);
  return node;
}

function detailPreview(item) {
  const path = mediaPath(item.mediaUrl);
  if (!previewCache || previewCache.itemId !== item.id || previewCache.path !== path) {
    previewCache = { itemId: item.id, path, node: buildPreview(item) };
  }
  return previewCache.node;
}

function renderDetail(item) {
  const preview = detailPreview(item);
  let source = null;
  try {
    source = new URL(item.media.sourceUrl).hostname.replace(/^www\./, '');
  } catch {
    source = null;
  }
  const shortcut = shortcutFor(item.id);
  const size = item.media.sizeBytes ? `${(item.media.sizeBytes / 1048576).toFixed(1).replace('.', ',')} Mo` : null;
  const added = new Date(item.createdAt);
  return h(
    'aside',
    { class: 'detail', 'aria-label': 'Détails du média' },
    h(
      'div',
      { class: 'detail-head' },
      h('h2', null, itemTitle(item)),
      h('button', { class: 'btn is-quiet', 'aria-label': 'Fermer les détails', onClick: () => selectItem(item.id) }, icon('close')),
    ),
    preview,
    can('play') ? h('button', { class: 'btn is-air is-wide', onClick: () => playItem(item) }, icon('play'), 'Jouer sur les overlays') : null,
    h(
      'dl',
      { class: 'facts' },
      item.message ? [h('dt', null, 'Message'), h('dd', null, item.message)] : null,
      [h('dt', null, 'Ajouté par'), h('dd', null, item.createdByName || 'Inconnu')],
      [h('dt', null, 'Le'), h('dd', null, Number.isNaN(added.getTime()) ? '—' : added.toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: 'numeric' }))],
      source ? [h('dt', null, 'Source'), h('dd', null, source)] : null,
      [h('dt', null, 'Format'), h('dd', null, [KIND_LABELS[item.media.kind] || item.media.kind, formatDuration(item.media.durationSec), size].filter(Boolean).join(', '))],
      can('play')
        ? [
            h('dt', null, 'Raccourci'),
            h(
              'dd',
              null,
              state.capturing
                ? h('span', { class: 'meta' }, 'Appuie sur la combinaison… (Échap pour annuler)')
                : shortcut
                  ? h('kbd', null, acceleratorLabel(shortcut))
                  : h('span', { class: 'meta' }, 'Aucun'),
            ),
          ]
        : null,
    ),
    h(
      'div',
      { class: 'row is-wrap' },
      can('play') ? h('button', { class: 'btn', onClick: () => startCapture(item) }, shortcut ? 'Changer le raccourci' : 'Définir un raccourci') : null,
      can('play') && shortcut ? h('button', { class: 'btn is-quiet', onClick: () => saveShortcut(item, null) }, 'Retirer') : null,
      item.canEdit ? h('button', { class: 'btn', onClick: () => openEditDialog(item) }, 'Modifier') : null,
      h('span', { class: 'spacer' }),
      item.canEdit ? h('button', { class: 'btn is-danger', onClick: () => openDeleteDialog(item) }, 'Supprimer') : null,
    ),
  );
}

// --- keyboard shortcuts (active guild) ---
let captureItem = null;

export function startCapture(item) {
  captureItem = item;
  state.capturing = true;
  renderView();
}

document.addEventListener('keydown', (event) => {
  if (!state.capturing || !captureItem) {
    return;
  }
  event.preventDefault();
  if (event.key === 'Escape') {
    state.capturing = false;
    renderView();
    return;
  }
  const accelerator = acceleratorFromEvent(event);
  if (accelerator === null) {
    return;
  }
  if (accelerator === undefined) {
    notify('Utilise Ctrl, Alt ou Maj avec une lettre, un chiffre ou une touche F.', true);
    return;
  }
  state.capturing = false;
  saveShortcut(captureItem, accelerator);
});

export async function saveShortcut(item, accelerator) {
  const bindings = {};
  for (const [existing, id] of Object.entries(state.bindings)) {
    if (id !== item.id && existing !== accelerator) {
      bindings[existing] = id;
    }
  }
  if (accelerator) {
    bindings[accelerator] = item.id;
  }
  const response = await run(() => invoke('media_board_set_bindings', { payload: { bindings } }));
  if (response) {
    state.bindings = response.bindings || {};
    if (response.failedAccelerators && response.failedAccelerators.length > 0) {
      notify('Ce raccourci est déjà pris par une autre application.', true);
    }
  }
  renderView();
}

// --- edit and delete ---
export function openEditDialog(item) {
  openDialog((close) => {
    const title = h('input', { class: 'field', maxlength: '240', value: item.title || '' });
    const message = h('textarea', { class: 'field', rows: '3', maxlength: '500', value: item.message || '' });
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          const body = await run(() => api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, { title: title.value, message: message.value }), 'Média modifié.');
          if (body) {
            close();
            loadItems();
          }
        },
      },
      h('label', null, 'Titre', title),
      h('label', null, 'Message', message),
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), h('button', { class: 'btn', type: 'submit' }, 'Enregistrer')),
    );
    return [h('h2', null, 'Modifier le média'), h('p', null, 'Un champ vide efface la valeur.'), form];
  });
}

export function openDeleteDialog(item) {
  confirmDialog('Supprimer ce média ?', `« ${itemTitle(item)} » sera retiré du board de la guilde.`, 'Supprimer', async () => {
    await run(() => api('DELETE', `/overlay/media-board/items/${encodeURIComponent(item.id)}`), 'Média supprimé.');
    state.selectedId = null;
    loadItems();
  });
}

// --- refresh hooks (no full re-render) ---
document.addEventListener('livechat:queue', () => {
  // The queue is the source of truth once it has been read again.
  refreshTriggerBadges();
});

document.addEventListener('livechat:uploads', () => {
  refreshUploadCards();
});

// --- dropping local files on the board ---
let dropVeil = null;
let dropDismissed = false;

function transfersFiles(event) {
  const transfer = event.dataTransfer;
  return Boolean(transfer && Array.from(transfer.types || []).includes('Files'));
}

function canDropFiles() {
  return Boolean(state.activeGuildId) && state.page === 'guild' && state.tab === 'board' && can('addMedia');
}

function inMainArea(target) {
  const main = document.querySelector('.main');
  return Boolean(main && target instanceof Node && main.contains(target));
}

function ensureDropVeil() {
  if (!dropVeil) {
    dropVeil = h(
      'div',
      { class: 'drop-veil', hidden: true, 'aria-hidden': 'true' },
      h('div', { class: 'drop-veil-inner' }, icon('plus'), h('p', null, 'Dépose tes fichiers pour les ajouter au board')),
    );
    document.body.append(dropVeil);
  }
  return dropVeil;
}

function showDropVeil() {
  if (!dropDismissed) {
    ensureDropVeil().hidden = false;
  }
}

function hideDropVeil() {
  if (dropVeil) {
    dropVeil.hidden = true;
  }
}

function resetDropVeil() {
  dropDismissed = false;
  hideDropVeil();
}

// Never let the WebView open a dropped file, anywhere in the window.
document.addEventListener('dragover', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  if (canDropFiles() && inMainArea(event.target)) {
    event.dataTransfer.dropEffect = 'copy';
    showDropVeil();
  }
});

document.addEventListener('dragenter', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  if (canDropFiles() && inMainArea(event.target)) {
    showDropVeil();
  }
});

// Only a real exit hides the veil: dragleave fired when moving between the
// board's children has a relatedTarget still inside the main area.
document.addEventListener('dragleave', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  const related = event.relatedTarget;
  if (!(related instanceof Node) || !inMainArea(related)) {
    resetDropVeil();
  }
});

document.addEventListener('drop', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  const files = event.dataTransfer && event.dataTransfer.files ? Array.from(event.dataTransfer.files) : [];
  const accepted = canDropFiles() && inMainArea(event.target);
  resetDropVeil();
  if (!accepted || files.length === 0) {
    return;
  }
  const guildId = state.activeGuildId;
  for (const file of files) {
    startFileUpload({ guildId, file, title: '', message: '' });
  }
});

document.addEventListener('dragend', resetDropVeil);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && dropVeil && !dropVeil.hidden) {
    dropDismissed = true;
    hideDropVeil();
  }
});
