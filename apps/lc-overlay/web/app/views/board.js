// Media board: grid, cards, detail panel, play, shortcuts, edit and delete.

import { api, run } from '../core/api.js';
import { openContextMenu } from '../core/contextmenu.js';
import { acceleratorFromEvent, acceleratorLabel, BARE_KEY_MESSAGE, cancelsCapture } from '../core/accelerators.js';
import { ARTWORK_MESSAGES, artworkExtension, droppedArtworkLink, fetchArtwork, MAX_ARTWORK_BYTES, pickArtworkFile } from '../core/artwork.js';
import {
  activeFilterCount,
  authorsOf,
  DEFAULT_FILTERS,
  droppedLink,
  DURATION_LABELS,
  selectItems,
  SORT_LABELS,
  SOURCE_LABELS,
} from '../core/board-filters.js';
import {
  artworkOf,
  cardClicked,
  cardHovered,
  CLOSED_VIEW,
  drawerUsed,
  mediaPath,
  pointerLeft,
  previewKey,
  pruneView,
  sameView,
  shownId,
} from '../core/detail-drawer.js';
import { h, icon, registerIcons } from '../core/dom.js';
import { formatDuration } from '../core/format.js';
import { cardRatio, cardStyle } from '../core/masonry.js';
import { invoke } from '../core/ipc.js';
import { activeGuild, can, describeError, KIND_LABELS, state } from '../core/state.js';
import { previewVolume } from '../core/sound.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { loadItems, refreshQueueAutomatically } from '../data.js';
import { openAddDialog } from './add-media.js';
import { openSayDialog } from './say.js';
import { canOpenEditor, openMediaEditor } from './editor/index.js';
import { openUploadMenu, renderUploadCard, startFileUpload, uploadsForGuild } from './uploads.js';
import { renderView } from './view.js';

registerIcons({
  star: 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
  filter: 'M4 6h16M7 12h10M10 18h4',
});

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
      label: state.selectedId === item.id && state.detailPinned ? 'Masquer les détails' : 'Afficher les détails',
      onSelect: () => selectItem(item.id),
    },
    canFavorite()
      ? { label: item.favorite ? 'Retirer des favoris' : 'Ajouter aux favoris', icon: 'star', onSelect: () => toggleFavorite(item) }
      : null,
    canOpenEditor(item) ? { label: 'Ouvrir dans l’éditeur…', icon: 'edit', onSelect: () => openMediaEditor(item, itemTitle(item)) } : null,
    item.canEdit ? { label: 'Modifier les infos…', onSelect: () => openEditDialog(item) } : null,
    can('play')
      ? {
          label: shortcutFor(item.id) ? 'Changer le raccourci' : 'Définir un raccourci',
          onSelect: () => {
            applyView({ selectedId: item.id, pinned: true, peekId: null });
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
  return selectItems(state.items, {
    search: state.search,
    kind: state.kind,
    filters: state.filters,
    shortcutItemIds: new Set(Object.values(state.bindings)),
    titleOf: itemTitle,
  });
}

// Favorites belong to a member: a device paired without one has none.
function canFavorite() {
  return Boolean(state.info && state.info.member && state.info.member.id);
}

export async function toggleFavorite(item) {
  const favorite = !item.favorite;
  const body = await run(() => api('PUT', `/overlay/media-board/items/${encodeURIComponent(item.id)}/favorite`, { favorite }));
  if (!body) {
    return;
  }
  for (const entry of state.items) {
    if (entry.id === item.id) {
      entry.favorite = favorite;
    }
  }
  const card = cards.get(item.id);
  if (card) {
    syncFavorite(card.node, favorite);
  }
  if (state.filters.favorites) {
    refreshGrid(false);
  }
  syncDetail();
}

function syncFavorite(card, favorite) {
  const star = card.querySelector('.fav');
  if (!star) {
    return;
  }
  star.classList.toggle('is-on', favorite);
  star.setAttribute('aria-pressed', String(favorite));
  star.title = favorite ? 'Retirer des favoris' : 'Ajouter aux favoris';
}

// --- card cache ---
// Card nodes live across renders, keyed by item id. Rebuilding them on every
// render (mute, filter, board reload) reloaded each thumbnail and made the
// grid flash. `entry.item` always points at the latest copy of the item, so
// the card's handlers never act on stale data.
const cards = new Map(); // itemId -> { node, key, item, loadedUrl }
// The previews of the pinned media and of the one being peeked, kept across
// renders of the view: rebuilding one reloaded its video and showed the
// loading spinner again. Coming back from a peek finds the pinned video where
// it was.
const previews = new Map(); // itemId -> { key, node }

function pausePreview(entry) {
  const media = entry.node.querySelector('audio, video');
  if (media && !media.paused) {
    media.pause();
  }
}

// Keeps only the previews the drawer can still show without a new hover.
function prunePreviews() {
  for (const [id, entry] of previews) {
    if (id !== state.selectedId && id !== peekId) {
      pausePreview(entry);
      previews.delete(id);
    }
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
    mediaPath(item.artwork && item.artwork.url),
    can('play'),
    canFavorite(),
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
  const artwork = kind === 'audio' ? artworkOf(item) : null;
  const mediaUrl = artwork ? artwork.url : typeof item.mediaUrl === 'string' && item.mediaUrl ? item.mediaUrl : null;
  const thumb = h(
    'button',
    {
      class: `thumb${artwork ? ' has-artwork' : ''}`,
      'data-media-url': mediaUrl,
      'data-kind': artwork ? 'image' : kind,
      'data-item-id': item.id,
      'aria-label': `Détails de ${itemTitle(item)}`,
      onClick: (event) => selectItem(entry.item.id, event.detail),
      // A double click plays the media, like the Play button. It leaves the
      // drawer as it was (see selectItem).
      onDblclick: () => {
        if (can('play')) {
          playItem(entry.item);
        }
      },
    },
    h('span', { class: 'glyph' }, icon(kind === 'audio' ? 'audio' : kind === 'image' ? 'image' : 'video')),
    artwork ? h('span', { class: 'kind-badge', 'aria-hidden': 'true' }, icon('audio')) : null,
    formatDuration(item.media.durationSec) ? h('span', { class: 'duration' }, formatDuration(item.media.durationSec)) : null,
  );
  if ((kind !== 'audio' || artwork) && mediaUrl) {
    thumbObserver.observe(thumb);
  }
  const ratio = artwork ? { width: artwork.width || 1, height: artwork.height || 1 } : item.media;
  const node = h(
    'div',
    {
      class: 'card',
      'data-item-id': item.id,
      // Height of the media's own shape (masonry, see core/masonry.js).
      style: cardStyle(cardRatio(ratio)),
      onContextmenu: (event) => openCardMenu(event, entry.item),
      onMouseenter: () => hoverDetail(entry.item.id),
      onMouseleave: leaveCard,
    },
    h(
      'div',
      { class: 'card-frame' },
      thumb,
      can('play')
        ? h('button', { class: 'play', 'aria-label': `Jouer ${itemTitle(item)}`, title: 'Jouer sur les overlays', onClick: () => playItem(entry.item) }, icon('play'))
        : null,
      canFavorite()
        ? h(
            'button',
            {
              class: 'fav',
              'aria-label': `Favori : ${itemTitle(item)}`,
              'aria-pressed': 'false',
              onClick: (event) => {
                event.stopPropagation();
                toggleFavorite(entry.item);
              },
            },
            icon('star'),
          )
        : null,
    ),
    h('p', { class: 'card-title', title: itemTitle(item) }, itemTitle(item)),
  );
  syncFavorite(node, Boolean(item.favorite));
  return node;
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
  syncFavorite(entry.node, Boolean(item.favorite));
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
  const selected = shownItem();
  // A preview no longer shown must not keep its video alive.
  prunePreviews();
  const filterCount = activeFilterCount(state.filters);
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
    h(
      'button',
      {
        class: `btn is-quiet filters-toggle${filterCount > 0 || state.filtersOpen ? ' is-active' : ''}`,
        'aria-expanded': String(state.filtersOpen),
        onClick: () => {
          state.filtersOpen = !state.filtersOpen;
          renderView();
        },
      },
      icon('filter'),
      filterCount > 0 ? `Filtres · ${filterCount}` : 'Filtres',
    ),
    h('span', { class: 'spacer' }),
    h('button', { class: 'btn is-quiet', title: 'Recharger le board', 'aria-label': 'Recharger le board', onClick: loadItems }, icon('refresh')),
    can('play') ? h('button', { class: 'btn', title: 'Faire lire un texte à voix haute sur les overlays', onClick: () => openSayDialog() }, icon('sound'), 'Dire') : null,
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
    content = h('p', { class: 'meta' }, 'Aucun média ne correspond à cette recherche ou à ces filtres.');
  } else {
    // Provisional cards always come first, whatever the search and filter.
    content = h('div', { class: 'grid', id: 'grid' }, uploads.map(uploadCard), items.map(renderCard));
  }
  return h(
    'div',
    null,
    toolbar,
    state.filtersOpen ? renderFilters() : null,
    h('div', { class: 'board' }, content, selected ? renderDetail(selected) : null),
  );
}

function setFilter(patch) {
  state.filters = { ...state.filters, ...patch };
  renderView();
}

function filterSelect(label, value, options, onChange) {
  return h(
    'label',
    { class: 'filter' },
    h('span', null, label),
    h(
      'select',
      { class: 'field', onChange: (event) => onChange(event.target.value) },
      options.map(([id, text]) => h('option', { value: id, selected: id === value }, text)),
    ),
  );
}

function renderFilters() {
  const filters = state.filters;
  const authors = authorsOf(state.items);
  return h(
    'div',
    { class: 'filters', role: 'group', 'aria-label': 'Filtres du board' },
    canFavorite()
      ? h(
          'label',
          { class: 'switch' },
          h('input', { type: 'checkbox', checked: filters.favorites, onChange: (event) => setFilter({ favorites: event.target.checked }) }),
          'Favoris',
        )
      : null,
    can('play')
      ? h(
          'label',
          { class: 'switch' },
          h('input', { type: 'checkbox', checked: filters.shortcut, onChange: (event) => setFilter({ shortcut: event.target.checked }) }),
          'Avec raccourci',
        )
      : null,
    filterSelect('Ajouté par', filters.author, [['', 'Tout le monde'], ...authors.map((name) => [name, name])], (author) => setFilter({ author })),
    filterSelect('Source', filters.source, [['', 'Toutes'], ...Object.entries(SOURCE_LABELS)], (source) => setFilter({ source })),
    filterSelect('Durée', filters.duration, [['', 'Toutes'], ...Object.entries(DURATION_LABELS)], (duration) => setFilter({ duration })),
    filterSelect('Trier', filters.sort, Object.entries(SORT_LABELS), (sort) => setFilter({ sort })),
    activeFilterCount(filters) > 0 || filters.sort !== DEFAULT_FILTERS.sort
      ? h('button', { class: 'btn is-quiet', onClick: () => setFilter({ ...DEFAULT_FILTERS }) }, 'Réinitialiser')
      : null,
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

// --- drawer: pinned by a click, following the pointer otherwise ---
// `state.selectedId` and `state.detailPinned` hold the selected media;
// `peekId` is another media looked at by hovering while one is pinned (see
// core/detail-drawer.js).
const HOVER_DELAY_MS = 450;
const LEAVE_DELAY_MS = 300;
let hoverTimer = null;
let leaveTimer = null;
let peekId = null;

function currentView() {
  return { selectedId: state.selectedId, pinned: state.detailPinned, peekId };
}

function applyView(view) {
  state.selectedId = view.selectedId;
  state.detailPinned = view.pinned;
  peekId = view.peekId;
}

// The media the drawer shows: the peeked one, else the selected one.
function shownItem() {
  applyView(pruneView(currentView(), (id) => state.items.some((item) => item.id === id)));
  const id = shownId(currentView());
  return (id && state.items.find((item) => item.id === id)) || null;
}

function clearDrawerTimers() {
  clearTimeout(hoverTimer);
  clearTimeout(leaveTimer);
  hoverTimer = null;
  leaveTimer = null;
}

// A mouse click waits this long before toggling the drawer, so that a double
// click (which plays the media) neither opens nor closes it.
const DOUBLE_CLICK_MS = 250;
let clickTimer = null;

// A click pins the drawer on a media (a second click on it closes it).
// `clickCount` is the `detail` of the click event, when there is one.
export function selectItem(id, clickCount) {
  clearTimeout(clickTimer);
  clickTimer = null;
  if (clickCount === 1) {
    clickTimer = setTimeout(() => {
      clickTimer = null;
      toggleDetail(id, 1);
    }, DOUBLE_CLICK_MS);
    return;
  }
  toggleDetail(id, clickCount);
}

function toggleDetail(id, clickCount) {
  clearDrawerTimers();
  applyView(cardClicked(currentView(), id, clickCount));
  state.capturing = false;
  syncDetail();
}

export function closeDetail() {
  clearDrawerTimers();
  applyView(CLOSED_VIEW);
  state.capturing = false;
  syncDetail();
}

// Hovering a card shows its details after a short delay, also while another
// media is pinned: the drawer then peeks at the hovered one.
function hoverDetail(id) {
  clearDrawerTimers();
  hoverTimer = setTimeout(() => {
    hoverTimer = null;
    const next = cardHovered(currentView(), id);
    if (state.capturing || sameView(next, currentView())) {
      return;
    }
    applyView(next);
    syncDetail();
  }, HOVER_DELAY_MS);
}

function leaveCard() {
  clearTimeout(hoverTimer);
  hoverTimer = null;
  scheduleHoverClose();
}

// Once the pointer has left the cards and the drawer, an unpinned drawer
// closes and a peek gives the drawer back to the pinned media.
function scheduleHoverClose() {
  clearTimeout(leaveTimer);
  leaveTimer = null;
  if (sameView(pointerLeft(currentView()), currentView())) {
    return;
  }
  leaveTimer = setTimeout(() => {
    leaveTimer = null;
    const next = pointerLeft(currentView());
    if (state.capturing || sameView(next, currentView())) {
      return;
    }
    applyView(next);
    syncDetail();
  }, LEAVE_DELAY_MS);
}

// Updates the drawer in place: the grid never moves and the preview (and its
// video) is reused while the same media stays shown.
function syncDetail() {
  const board = document.querySelector('.view .board');
  if (!board) {
    renderView();
    return;
  }
  const selected = shownItem();
  prunePreviews();
  const current = board.querySelector(':scope > .detail');
  const next = selected ? renderDetail(selected) : null;
  if (current && next) {
    // A refresh of the same media (a playback badge, a favorite) must not
    // replay the drawer's entry animation.
    if (current.dataset.itemId === next.dataset.itemId) {
      next.classList.add('is-refresh');
    }
    current.replaceWith(next);
  } else if (current) {
    current.remove();
  } else if (next) {
    board.append(next);
  }
  syncSelectedCards();
}

// The highlighted card is the selected media, not the one being peeked.
function syncSelectedCards() {
  for (const [id, entry] of cards) {
    entry.node.classList.toggle('is-selected', id === state.selectedId);
  }
}

// The drawer floats over the view, between the header and the footer.
function placeDrawer(node) {
  const view = document.getElementById('view');
  if (!view || !node.isConnected) {
    return;
  }
  const box = view.getBoundingClientRect();
  node.style.top = `${Math.round(box.top + 12)}px`;
  node.style.bottom = `${Math.round(Math.max(12, window.innerHeight - box.bottom + 12))}px`;
}

// The view shrinks when the on-air bar shows up (a playback started after the
// drawer opened): follow its box, not only the window's.
function replaceDrawer() {
  const node = document.querySelector('.view .detail');
  if (node) {
    placeDrawer(node);
  }
}

window.addEventListener('resize', replaceDrawer);
const viewNode = document.getElementById('view');
if (viewNode && typeof ResizeObserver === 'function') {
  new ResizeObserver(replaceDrawer).observe(viewNode);
}

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && state.selectedId && !state.capturing && !document.getElementById('dialog-root').firstElementChild) {
    closeDetail();
  }
});

export async function playItem(item) {
  const guild = activeGuild();
  if (guild && guild.membership.muted) {
    notify('Guilde en pause : reprends-la pour jouer.', true);
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
    // An audio shows its picture; without one (or when it cannot be loaded)
    // it shows an audio mark above the player, never an empty image.
    const glyph = h('span', { class: 'preview-glyph', 'aria-hidden': 'true' }, icon('audio'));
    const artwork = artworkOf(item);
    if (artwork) {
      node.classList.add('has-artwork');
      const picture = h('img', { class: 'preview-artwork', src: artwork.url, alt: '' });
      picture.addEventListener('error', () => {
        node.classList.remove('has-artwork');
        picture.replaceWith(glyph);
      });
      node.append(picture);
    } else {
      node.append(glyph);
    }
    const audio = h('audio', { src: url, controls: true, preload: 'metadata' });
    audio.volume = previewVolume(state.activeGuildId);
    node.append(audio);
    return node;
  }
  if (kind === 'video') {
    // The native controls only appear once the first frame is there: shown
    // earlier, they play their loading spinner for the few milliseconds the
    // video takes to load.
    const video = h('video', { src: url, preload: 'auto', playsinline: true });
    video.volume = previewVolume(state.activeGuildId);
    video.addEventListener('loadedmetadata', () => adoptRatio(video.videoWidth, video.videoHeight));
    node.append(video);
    const still = cardStill(item.id);
    if (still) {
      node.append(still);
    }
    const reveal = () => {
      video.controls = true;
      still?.remove();
    };
    video.addEventListener('loadeddata', reveal, { once: true });
    video.addEventListener('error', reveal, { once: true });
    return node;
  }
  const image = h('img', { src: url, alt: '' });
  image.addEventListener('load', () => adoptRatio(image.naturalWidth, image.naturalHeight));
  node.append(image);
  return node;
}

function detailPreview(item) {
  const key = previewKey(item);
  let entry = previews.get(item.id);
  if (!entry || entry.key !== key) {
    if (entry) {
      pausePreview(entry);
    }
    entry = { key, node: buildPreview(item) };
    previews.set(item.id, entry);
  }
  // A preview out of the drawer (the pinned one during a peek) goes silent
  // and keeps its position.
  for (const [id, other] of previews) {
    if (id !== item.id) {
      pausePreview(other);
    }
  }
  return entry.node;
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
  const drawer = h(
    'aside',
    {
      class: `detail${state.detailPinned && state.selectedId === item.id ? ' is-pinned' : ' is-peek'}`,
      'data-item-id': item.id,
      'aria-label': 'Détails du média',
      onMouseenter: () => clearTimeout(leaveTimer),
      onMouseleave: scheduleHoverClose,
      // Using the drawer pins the media it shows. Only classes change: the
      // drawer must stay in place for the click that follows.
      onPointerdown: () => {
        const next = drawerUsed(currentView());
        if (!sameView(next, currentView())) {
          clearDrawerTimers();
          applyView(next);
          drawer.classList.replace('is-peek', 'is-pinned');
          syncSelectedCards();
          prunePreviews();
        }
      },
    },
    h(
      'div',
      { class: 'detail-head' },
      h('h2', null, itemTitle(item)),
      canFavorite()
        ? h(
            'button',
            {
              class: `btn is-quiet fav-toggle${item.favorite ? ' is-on' : ''}`,
              'aria-pressed': String(Boolean(item.favorite)),
              title: item.favorite ? 'Retirer des favoris' : 'Ajouter aux favoris',
              onClick: () => toggleFavorite(item),
            },
            icon('star'),
          )
        : null,
      h('button', { class: 'btn is-quiet', 'aria-label': 'Fermer les détails', onClick: closeDetail }, icon('close')),
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
                ? h('span', { class: 'meta' }, 'Appuie sur la combinaison… (Échap seul pour annuler)')
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
      canOpenEditor(item) ? h('button', { class: 'btn', title: 'Découper, recadrer, dessiner…', onClick: () => openMediaEditor(item, itemTitle(item)) }, icon('edit'), 'Éditer') : null,
      item.canEdit ? h('button', { class: 'btn', onClick: () => openEditDialog(item) }, 'Infos') : null,
      h('span', { class: 'spacer' }),
      item.canEdit ? h('button', { class: 'btn is-danger', onClick: () => openDeleteDialog(item) }, 'Supprimer') : null,
    ),
  );
  queueMicrotask(() => placeDrawer(drawer));
  return drawer;
}

// The preview follows the volume sliders while it plays.
document.addEventListener('livechat:sound', () => {
  for (const entry of previews.values()) {
    const media = entry.node.querySelector('audio, video');
    if (media) {
      media.volume = previewVolume(state.activeGuildId);
    }
  }
});

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
  if (cancelsCapture(event)) {
    state.capturing = false;
    renderView();
    return;
  }
  const accelerator = acceleratorFromEvent(event);
  if (accelerator === null) {
    return;
  }
  if (accelerator === undefined) {
    notify(BARE_KEY_MESSAGE, true);
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
async function uploadArtwork(item, file) {
  if (file.size > MAX_ARTWORK_BYTES) {
    notify(ARTWORK_MESSAGES.tooLarge, true);
    return false;
  }
  const bytes = new Uint8Array(await file.arrayBuffer());
  const response = await run(() =>
    invoke('media_artwork_upload', bytes, {
      headers: {
        'x-guild-id': state.activeGuildId,
        'x-item-id': item.id,
        // Headers are ASCII: only the extension of the name is kept.
        'x-filename': `artwork${artworkExtension(file)}`,
        'x-content-type': file.type || 'application/octet-stream',
      },
    }),
  );
  if (!response) {
    return false;
  }
  if (!response.ok) {
    const code = response.body && response.body.error;
    notify(describeError(code, 'Image refusée par le serveur.'), true);
    return false;
  }
  return true;
}

export function openEditDialog(item) {
  openDialog((close) => {
    const title = h('input', { class: 'field', maxlength: '240', value: item.title || '' });
    const message = h('textarea', { class: 'field', rows: '3', maxlength: '500', value: item.message || '' });
    const isAudio = item.media.kind === 'audio';
    let chosen = null;
    let removeArtwork = false;
    let previewUrl = null;
    let importing = false;
    const current = artworkOf(item);
    const artworkPreview = h('img', { class: 'artwork-preview', alt: '', draggable: 'false', hidden: !current, src: current ? current.url : null });
    // Without a picture the zone shows an audio mark, never an empty image.
    const artworkEmpty = h('span', { class: 'artwork-preview is-empty', 'aria-hidden': 'true', hidden: Boolean(current) }, icon('audio'));
    const artworkName = h('span', { class: 'meta' }, current ? 'Image actuelle' : 'Aucune image');
    const submit = h('button', { class: 'btn', type: 'submit' }, 'Enregistrer');
    const showPicture = (shown) => {
      artworkPreview.hidden = !shown;
      artworkEmpty.hidden = shown;
      removeButton.hidden = !shown;
    };
    // The picture that will be sent on « Enregistrer ».
    const choose = (file, label) => {
      chosen = file;
      removeArtwork = false;
      if (previewUrl) {
        URL.revokeObjectURL(previewUrl);
      }
      previewUrl = URL.createObjectURL(file);
      artworkPreview.src = previewUrl;
      artworkName.textContent = label || file.name;
      showPicture(true);
    };
    // Chosen, dropped or pasted files: true when one of them was handled.
    const chooseFrom = (files) => {
      const picked = pickArtworkFile(files);
      if (!picked) {
        return false;
      }
      if (picked.error) {
        notify(picked.error, true);
      } else {
        choose(picked.file);
      }
      return true;
    };
    // A link is downloaded here, in the WebView, then sent like a file.
    const importLink = async (link) => {
      if (importing) {
        return;
      }
      importing = true;
      const label = artworkName.textContent;
      artworkName.textContent = 'Téléchargement de l’image…';
      importButton.disabled = true;
      submit.disabled = true;
      const result = await fetchArtwork(link);
      importing = false;
      importButton.disabled = false;
      submit.disabled = false;
      if (result.error) {
        artworkName.textContent = label;
        notify(result.error, true);
        return;
      }
      linkField.value = '';
      choose(result.file);
    };
    const artworkInput = h('input', {
      type: 'file',
      accept: 'image/*',
      hidden: true,
      onChange: () => {
        chooseFrom(artworkInput.files);
        artworkInput.value = '';
      },
    });
    const removeButton = h(
      'button',
      {
        class: 'btn is-quiet',
        type: 'button',
        hidden: !current,
        onClick: () => {
          chosen = null;
          removeArtwork = true;
          artworkName.textContent = 'Aucune image';
          showPicture(false);
        },
      },
      'Retirer',
    );
    const linkField = h('input', {
      class: 'field',
      type: 'url',
      placeholder: 'https://…',
      'aria-label': 'Lien d’une image',
      onKeydown: (event) => {
        // Enter imports the link instead of saving the dialog.
        if (event.key === 'Enter') {
          event.preventDefault();
          importLink(linkField.value);
        }
      },
    });
    const importButton = h('button', { class: 'btn', type: 'button', onClick: () => importLink(linkField.value) }, 'Importer');
    const artworkZone = h(
      'div',
      { class: 'artwork-drop' },
      artworkPreview,
      artworkEmpty,
      h(
        'div',
        { class: 'stack' },
        artworkName,
        h('div', { class: 'row' }, h('button', { class: 'btn', type: 'button', onClick: () => artworkInput.click() }, 'Choisir une image'), removeButton),
      ),
      artworkInput,
    );
    const onOverlay = h('input', { type: 'checkbox', checked: Boolean(item.artworkOnOverlay) });
    const carriesPicture = (event) => {
      const types = Array.from((event.dataTransfer && event.dataTransfer.types) || []);
      return types.includes('Files') || types.includes('text/uri-list') || types.includes('text/html') || types.includes('text/plain');
    };
    // A drop on the picture block belongs to the picture: it stops here and
    // never reaches the board's own drop handlers (which import new media).
    const onDrag = (event) => {
      if (!carriesPicture(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = 'copy';
      artworkZone.classList.add('is-over');
    };
    const artworkFields = isAudio
      ? h(
          'div',
          {
            class: 'stack artwork-fields',
            onDragenter: onDrag,
            onDragover: onDrag,
            onDragleave: (event) => {
              if (!(event.relatedTarget instanceof Node) || !artworkFields.contains(event.relatedTarget)) {
                artworkZone.classList.remove('is-over');
              }
            },
            onDrop: (event) => {
              if (!carriesPicture(event)) {
                return;
              }
              event.preventDefault();
              event.stopPropagation();
              artworkZone.classList.remove('is-over');
              const transfer = event.dataTransfer;
              if (chooseFrom(transfer.files)) {
                return;
              }
              const link = droppedArtworkLink({
                html: transfer.getData('text/html'),
                uriList: transfer.getData('text/uri-list'),
                text: transfer.getData('text/plain'),
              });
              if (link) {
                importLink(link);
              } else {
                notify(ARTWORK_MESSAGES.nothingDropped, true);
              }
            },
            onPaste: (event) => {
              const data = event.clipboardData;
              if (!data) {
                return;
              }
              // A copied image is used as it is; a copied link is imported.
              if (chooseFrom(data.files)) {
                event.preventDefault();
                return;
              }
              const link = droppedArtworkLink({ text: data.getData('text/plain') });
              if (link) {
                event.preventDefault();
                linkField.value = link;
                importLink(link);
              }
            },
          },
          h('span', { class: 'field-label' }, 'Image'),
          artworkZone,
          h('p', { class: 'artwork-hint' }, 'Dépose une image ou un lien ici, ou colle le lien d’une image :'),
          h('div', { class: 'artwork-link' }, linkField, importButton),
          h('label', { class: 'switch' }, onOverlay, 'Afficher l’image sur les overlays pendant la lecture'),
        )
      : null;
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          if (importing) {
            return;
          }
          const patch = { title: title.value, message: message.value };
          if (isAudio) {
            patch.artworkOnOverlay = onOverlay.checked;
          }
          const body = await run(() => api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, patch));
          if (!body) {
            return;
          }
          if (chosen && !(await uploadArtwork(item, chosen))) {
            return;
          }
          if (removeArtwork && current) {
            const removed = await run(() => api('DELETE', `/overlay/media-board/items/${encodeURIComponent(item.id)}/artwork`));
            if (!removed) {
              return;
            }
          }
          notify('Média modifié.');
          close();
          loadItems();
        },
      },
      h('label', null, 'Titre', title),
      h('label', null, 'Message', message),
      artworkFields,
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, 'Infos du média'), h('p', null, 'Un champ vide efface la valeur.'), form];
  });
}

export function openDeleteDialog(item) {
  confirmDialog('Supprimer ce média ?', `« ${itemTitle(item)} » sera retiré du board de la guilde.`, 'Supprimer', async () => {
    await run(() => api('DELETE', `/overlay/media-board/items/${encodeURIComponent(item.id)}`), 'Média supprimé.');
    applyView(CLOSED_VIEW);
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

// A drag started inside the board (a thumbnail, selected text) is not an
// import.
let internalDrag = false;
document.addEventListener('dragstart', () => {
  internalDrag = true;
});
document.addEventListener('dragend', () => {
  internalDrag = false;
});

function transfersFiles(event) {
  const transfer = event.dataTransfer;
  if (!transfer || internalDrag) {
    return false;
  }
  const types = Array.from(transfer.types || []);
  // A link dragged from a browser (or a text holding one) opens the import
  // dialog; files are uploaded directly.
  return types.includes('Files') || types.includes('text/uri-list') || types.includes('text/plain');
}

function droppedFiles(event) {
  return event.dataTransfer && event.dataTransfer.files ? Array.from(event.dataTransfer.files) : [];
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
      h('div', { class: 'drop-veil-inner' }, icon('plus'), h('p', null, 'Dépose un fichier ou un lien pour l’ajouter au board')),
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
  const files = droppedFiles(event);
  const accepted = canDropFiles() && inMainArea(event.target);
  resetDropVeil();
  if (!accepted) {
    return;
  }
  if (files.length === 0) {
    const transfer = event.dataTransfer;
    const text = transfer.getData('text/uri-list') || transfer.getData('text/plain');
    if (text && text.trim()) {
      openAddDialog({ url: droppedLink(text) || '' });
    }
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
