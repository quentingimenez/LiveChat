// Media board: grid, cards, detail panel, play, shortcuts, edit and delete.

import { api, run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { acceleratorFromEvent, acceleratorLabel } from '../core/accelerators.js';
import { h, icon } from '../core/dom.js';
import { formatDuration } from '../core/format.js';
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

// Thumbnails load their media only when they come close to the viewport.
const thumbObserver = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) {
        continue;
      }
      thumbObserver.unobserve(entry.target);
      const { mediaUrl, kind } = entry.target.dataset;
      if (!mediaUrl) {
        return;
      }
      const media =
        kind === 'video'
          ? h('video', { src: `${mediaUrl}#t=0.1`, preload: 'metadata', muted: true, playsinline: true, tabindex: '-1' })
          : h('img', { src: mediaUrl, alt: '', loading: 'lazy' });
      // The fallback glyph goes away as soon as the thumbnail can be shown.
      const glyph = entry.target.querySelector('.glyph');
      media.addEventListener('error', () => media.remove());
      media.addEventListener(kind === 'video' ? 'loadeddata' : 'load', () => glyph && glyph.remove());
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

function badgeNode(badge) {
  return badge ? h('span', { class: `card-badge${badge.air ? ' is-air' : ''}` }, badge.label) : null;
}

// Updates badges in place: re-rendering the whole view would reload the video
// in the detail panel.
function refreshTriggerBadges() {
  if (state.tab !== 'board' || state.page !== 'guild') {
    return;
  }
  pruneTriggers();
  for (const card of document.querySelectorAll('.card[data-item-id]')) {
    const frame = card.querySelector('.card-frame');
    if (!frame) {
      continue;
    }
    const badge = triggerBadgeFor(card.dataset.itemId);
    let node = frame.querySelector('.card-badge');
    if (!badge) {
      if (node) {
        node.remove();
      }
      continue;
    }
    if (!node) {
      node = h('span', { class: 'card-badge' });
      frame.append(node);
    }
    node.className = `card-badge${badge.air ? ' is-air' : ''}`;
    node.textContent = badge.label;
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
  const cards = uploadCards();
  for (const node of grid.querySelectorAll('.card.is-upload')) {
    node.remove();
  }
  if (cards.length === 0 && grid.children.length === 0) {
    renderView();
    return;
  }
  grid.prepend(...cards);
}

function renderCard(item) {
  const kind = item.media.kind;
  const mediaUrl = typeof item.mediaUrl === 'string' && item.mediaUrl ? item.mediaUrl : null;
  const thumb = h(
    'button',
    {
      class: 'thumb',
      'data-media-url': mediaUrl,
      'data-kind': kind,
      'aria-label': `Détails de ${itemTitle(item)}`,
      onClick: () => selectItem(item.id),
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
      class: `card${state.selectedId === item.id ? ' is-selected' : ''}`,
      'data-item-id': item.id,
      onContextmenu: (event) => openCardMenu(event, item),
    },
    h(
      'div',
      { class: 'card-frame' },
      thumb,
      can('play')
        ? h('button', { class: 'play', 'aria-label': `Jouer ${itemTitle(item)}`, title: 'Jouer sur les overlays', onClick: () => playItem(item) }, icon('play'))
        : null,
      badgeNode(triggerBadgeFor(item.id)),
    ),
    h('p', { class: 'card-title', title: itemTitle(item) }, itemTitle(item)),
  );
}

export function renderBoard() {
  if (state.info && !can('viewBoard')) {
    return h('div', { class: 'empty' }, h('h2', null, 'Board réservé aux membres'), h('p', null, "Cette guilde ne montre pas son board aux invités. Tu reçois quand même les médias diffusés."));
  }
  const items = visibleItems();
  const uploads = activeUploads();
  const selected = state.items.find((item) => item.id === state.selectedId) || null;
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
        refreshGrid();
      },
    }),
    h(
      'div',
      { class: 'chips' },
      Object.entries(KIND_LABELS).map(([id, label]) =>
        h('button', { class: `chip${state.kind === id ? ' is-active' : ''}`, onClick: () => { state.kind = id; renderView(); } }, label),
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

function refreshGrid() {
  const grid = document.getElementById('grid');
  const items = visibleItems();
  const uploads = activeUploads();
  if (!grid || (items.length === 0 && uploads.length === 0)) {
    const input = document.activeElement;
    const position = input && input.selectionStart;
    renderView();
    const field = document.querySelector('.toolbar input');
    if (field) {
      field.focus();
      field.setSelectionRange(position ?? field.value.length, position ?? field.value.length);
    }
    return;
  }
  grid.replaceChildren(...uploads.map(uploadCard), ...items.map(renderCard));
}

export function selectItem(id) {
  state.selectedId = state.selectedId === id ? null : id;
  state.capturing = false;
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

function renderDetail(item) {
  const preview = h('div', { class: 'preview' });
  if (item.mediaUrl) {
    if (item.media.kind === 'video') {
      preview.append(h('video', { src: item.mediaUrl, controls: true, preload: 'metadata' }));
    } else if (item.media.kind === 'audio') {
      preview.append(h('audio', { src: item.mediaUrl, controls: true }));
    } else {
      preview.append(h('img', { src: item.mediaUrl, alt: '' }));
    }
  } else {
    preview.append(h('span', { class: 'meta' }, 'Média indisponible.'));
  }
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
