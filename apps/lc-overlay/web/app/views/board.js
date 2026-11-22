// Media board: grid, cards, detail panel, play, shortcuts, edit and delete.

import { api, run } from '../core/api.js';
import { performAction } from '../core/action-ui.js';
import { isEditable, openContextMenu } from '../core/contextmenu.js';
import { findAction } from '../core/find-shortcut.js';
import { acceleratorFromEvent, ALTGR_HINT, BARE_KEY_MESSAGE, UNSUPPORTED_KEY, UNSUPPORTED_KEY_MESSAGE, cancelsCapture } from '../core/accelerators.js';
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
  cardPicture,
  cardPreviewAt,
  cardHovered,
  CLOSED_VIEW,
  drawerUsed,
  mediaPath,
  pictureKey,
  pointerLeft,
  previewKey,
  pruneView,
  realignPreviewAt,
  sameView,
  shownId,
  thumbnailOf,
  withVersion,
} from '../core/detail-drawer.js';
import { h, icon, registerIcons } from '../core/dom.js';
import { formatBoardUsage, formatDuration, formatBytes, INVALID_DURATION_MESSAGE, MAX_DISPLAY_DURATION_SEC, parseDisplayDuration } from '../core/format.js';
import { cardRatio, cardStyle } from '../core/masonry.js';
import { pastedMediaFiles } from '../core/media-files.js';
import { badgeForItem } from '../core/queue-badges.js';
import {
  cardClickAction,
  copiedLinksSummary,
  deletableIds,
  deleteDialogText,
  deleteSummary,
  escapeAction,
  favoritePlan,
  importTargets,
  menuTargetIds,
  originLinks,
  playSummary,
  selectAllAction,
  selectAllToggle,
  selectRange,
  sourceLink,
  toggleId,
} from '../core/selection.js';
import { capturedKeyLabel, forgetShortcutLabel, shortcutLabel, writeShortcutLabel } from '../core/shortcut-labels.js';
import { invoke, listen } from '../core/ipc.js';
import { activeGuild, can, describeError, KIND_LABELS, state } from '../core/state.js';
import { previewVolume } from '../core/sound.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { loadItems, refreshQueueAutomatically } from '../data.js';
import { openAddDialog } from './add-media.js';
import { openImportDialog } from './import-media.js';
import { openSayDialog } from './say.js';
import { canOpenEditor, openMediaEditor } from './editor/index.js';
import { openUploadMenu, renderUploadCard, startFileUpload, uploadsForGuild } from './uploads.js';
import { renderView } from './view.js';

registerIcons({
  // The star's ink is bottom heavy: its area centroid sits 1.26 units below the
  // centre of its box (y 3.09..19.19), so the glyph is raised by that much to
  // read centred in the circle of the favourite button.
  star: 'M12 3.09l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
  filter: 'M4 6h16M7 12h10M10 18h4',
  check: 'M5 12.5l4.5 4.5L19 7',
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
  const link = sourceLink(item);
  openContextMenu(event, [
    { heading: itemTitle(item) },
    can('play') ? { label: 'Jouer sur les overlays', icon: 'play', onSelect: () => playItem(item) } : null,
    {
      label: state.selectedId === item.id && state.detailPinned ? 'Masquer' : 'Détails',
      onSelect: () => selectItem(item.id),
    },
    canFavorite()
      ? { label: item.favorite ? 'Retirer' : 'Favori', icon: 'star', onSelect: () => toggleFavorite(item) }
      : null,
    canOpenEditor(item) ? { label: 'Modifier', icon: 'edit', onSelect: () => openMediaEditor(item, itemTitle(item)) } : null,
    { label: 'Infos…', onSelect: () => openInfoDialog(item) },
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
    link ? { label: 'Copier le lien d’origine', icon: 'copy', onSelect: () => copyText(link, 'Lien copié.') } : null,
    importTargets(state.guilds, state.activeGuildId).length > 0
      ? { label: 'Ajouter à une autre guilde…', onSelect: () => openImportDialog(menuItemIds(item), leaveSelection) }
      : null,
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

function paintFavorite(id, favorite) {
  applyFavorite(id, favorite);
  if (state.filters.favorites) {
    refreshGrid(false);
  }
  syncDetail();
}

export async function toggleFavorite(item, control) {
  const previous = Boolean(item.favorite);
  const favorite = !previous;
  await performAction(
    control,
    `favorite:${item.id}`,
    () => api('PUT', `/overlay/media-board/items/${encodeURIComponent(item.id)}/favorite`, { favorite }),
    {
      // The star flips at once; a refusal puts it back.
      optimistic: () => paintFavorite(item.id, favorite),
      rollback: () => paintFavorite(item.id, previous),
      onSuccess: (body) => {
        const confirmed = body && typeof body.favorite === 'boolean' ? body.favorite : favorite;
        if (confirmed !== favorite) {
          paintFavorite(item.id, confirmed);
        }
      },
    },
  );
}

// Applies a favorite change to the item copies, its card and the drawer.
function applyFavorite(id, favorite) {
  for (const entry of state.items) {
    if (entry.id === id) {
      entry.favorite = favorite;
    }
  }
  const card = cards.get(id);
  if (card) {
    syncFavorite(card.node, favorite);
  }
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
    shownDuration(item),
    item.media.width ?? null,
    item.media.height ?? null,
    mediaPath(item.mediaUrl),
    mediaPath(item.artwork && item.artwork.url),
    // The picture keeps its URL when the editor replaces it: its version is
    // what tells a new one from the cached one (see cardPicture).
    pictureKey(item),
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
      const { mediaUrl, kind, itemId, previewAt } = entry.target.dataset;
      if (!mediaUrl) {
        continue;
      }
      // A video without a picture of its own is seeked a little way in: its
      // first frame is often black (a fade-in), so `#t=0` showed a black card.
      const seek = Number.parseFloat(previewAt);
      const media =
        kind === 'video'
          ? h('video', {
              src: Number.isFinite(seek) ? `${mediaUrl}#t=${seek}` : mediaUrl,
              preload: 'metadata',
              muted: true,
              playsinline: true,
              tabindex: '-1',
              draggable: 'false',
            })
          : h('img', { src: mediaUrl, alt: '', loading: 'lazy', draggable: 'false' });
      // The fallback glyph goes away as soon as the thumbnail can be shown.
      const glyph = entry.target.querySelector('.glyph');
      media.addEventListener('error', () => media.remove());
      if (kind === 'video' && Number.isFinite(seek)) {
        // The announced length can be stale: seeking past the real end paints
        // a black card, so the seek moves back inside the video once known.
        media.addEventListener('loadedmetadata', () => {
          const at = realignPreviewAt(seek, media.duration);
          if (at !== null) {
            media.currentTime = at;
          }
        });
      }
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
// `state.queue` says which board item is on air or waiting; `state.triggers`
// keeps the job returned by a click until a queue read confirms it.
function pruneTriggers() {
  const queueAt = state.queue.at || 0;
  const { playing, pending } = state.queue;
  const inQueue = (jobId) =>
    Boolean(playing && playing.id === jobId) || (pending || []).some((job) => job.id === jobId);
  for (const [jobId, hint] of state.triggerHints) {
    if (queueAt <= hint.at || inQueue(jobId)) {
      continue;
    }
    state.triggerHints.delete(jobId);
    state.triggers.delete(jobId);
  }
}

function triggerBadgeFor(itemId) {
  pruneTriggers();
  const item = state.items.find((candidate) => candidate.id === itemId) || null;
  return badgeForItem(state.queue, state.triggers, state.triggerHints, item);
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
  // A video with a thumbnail shows that image instead of loading the video.
  const picture = cardPicture(item);
  const previewAt = picture.kind === 'video' ? cardPreviewAt(item) : null;
  const thumb = h(
    'button',
    {
      class: `thumb${artwork ? ' has-artwork' : ''}`,
      'data-media-url': picture.url,
      'data-kind': picture.kind,
      'data-preview-at': previewAt === null ? '' : String(previewAt),
      'data-item-id': item.id,
      'aria-label': `Détails de ${itemTitle(item)}`,
      onClick: (event) => {
        const action = cardClickAction({
          selecting: state.selecting,
          shiftKey: event.shiftKey,
          ctrlKey: event.ctrlKey,
          metaKey: event.metaKey,
        });
        if (action === 'details') {
          selectItem(entry.item.id, event.detail);
        } else if (action === 'pick') {
          pickCard(entry.item.id);
        } else {
          pickItem(entry.item.id, action === 'range');
        }
      },
      // A double click plays the media, like the Play button. It leaves the
      // drawer as it was (see selectItem).
      onDblclick: (event) => {
        if (can('play') && !state.selecting) {
          playItem(entry.item, event.currentTarget);
        }
      },
    },
    h('span', { class: 'glyph' }, icon(kind === 'audio' ? 'audio' : kind === 'image' ? 'image' : 'video')),
    h('span', { class: 'pick', 'aria-hidden': 'true' }, icon('check')),
    artwork ? h('span', { class: 'kind-badge', 'aria-hidden': 'true' }, icon('audio')) : null,
    formatDuration(shownDuration(item)) ? h('span', { class: 'duration' }, formatDuration(shownDuration(item))) : null,
  );
  if (picture.url) {
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
      // The thumbnail and the card's own buttons handle their click; the
      // title and the card's background open the drawer too (or pick the
      // media in the selection mode).
      onClick: (event) => {
        if (event.target.closest('button')) {
          return;
        }
        const action = cardClickAction({
          selecting: state.selecting,
          shiftKey: event.shiftKey,
          ctrlKey: event.ctrlKey,
          metaKey: event.metaKey,
        });
        if (action === 'details') {
          selectItem(entry.item.id, event.detail);
        } else if (action === 'pick') {
          pickCard(entry.item.id);
        } else {
          pickItem(entry.item.id, action === 'range');
        }
      },
      onMouseenter: () => {
        if (!state.selecting && hoverDetails) {
          hoverDetail(entry.item.id);
        }
      },
      onMouseleave: leaveCard,
    },
    h(
      'div',
      { class: 'card-frame' },
      thumb,
      can('play')
        ? h(
            'button',
            {
              class: 'play',
              'aria-label': `Jouer ${itemTitle(item)}`,
              title: 'Jouer sur les overlays',
              onClick: (event) => {
                event.stopPropagation();
                playItem(entry.item, event.currentTarget);
              },
            },
            icon('play'),
          )
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
                toggleFavorite(entry.item, event.currentTarget);
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
    const picture = cardPicture(item);
    if (thumb && !thumb.querySelector('img, video') && picture.url) {
      thumb.dataset.mediaUrl = picture.url;
      // The length may arrive with this payload (an import in progress), and
      // it is where the card seeks its video.
      const previewAt = picture.kind === 'video' ? cardPreviewAt(item) : null;
      thumb.dataset.previewAt = previewAt === null ? '' : String(previewAt);
    }
  }
  entry.node.classList.toggle('is-selected', state.selectedId === item.id);
  entry.node.classList.toggle('is-picked', state.selection.has(item.id));
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

// Common shapes of a board (portrait phone, landscape, square, audio wide).
// The skeleton reuses the same masonry spans and the same thumbnail + title
// anatomy as a real card: it is a placeholder, not a different kind of tile.
const SKELETON_RATIOS = [
  16 / 9,
  1,
  2,
  9 / 16,
  3 / 2,
  9 / 16,
  2,
  1,
  9 / 16,
  2,
  16 / 10,
  3 / 4,
];

// A last-visit layout can describe up to 60 media; only the first screenful is
// painted, so the shimmer never animates dozens of off-screen gradients.
const MAX_SKELETON_CARDS = 24;

// The shapes of the guild's last visit when the persistent layout cache has
// them, else the generic distribution above.
function skeletonRatios(count) {
  const layout = Array.isArray(state.boardLayout) ? state.boardLayout : null;
  if (layout && layout.length > 0) {
    return layout
      .slice(0, MAX_SKELETON_CARDS)
      .map((media) => cardRatio(media));
  }
  return Array.from(
    { length: count },
    (_, index) => SKELETON_RATIOS[index % SKELETON_RATIOS.length],
  );
}

// First load: the grid's shape is painted immediately, so the board no longer
// looks like an empty guild while the first request is in flight.
function boardSkeletonCards(count = SKELETON_RATIOS.length) {
  return skeletonRatios(count).map((ratio) => {
    return h(
      'div',
      {
        class: 'card is-skeleton',
        style: cardStyle(ratio),
        'aria-hidden': 'true',
      },
      h('div', { class: 'card-frame' }, h('span', { class: 'skeleton-block' })),
      h(
        'div',
        { class: 'card-title-skeleton' },
        h('span', { class: 'skeleton-block' }),
        h('span', { class: 'skeleton-block is-short' }),
      ),
    );
  });
}

function retryItems() {
  if (!state.itemsError) {
    return;
  }
  state.itemsLoaded = false;
  state.itemsError = null;
  renderView();
  loadItems({ quiet: true });
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
  // Occupation du board (`GET /overlay/guild`): hidden while unknown, e.g. for
  // a role without the `viewBoard` right.
  const mediaBoard = state.info && state.info.guild ? state.info.guild.mediaBoard : null;
  const boardUsage = formatBoardUsage(mediaBoard);
  const toolbar = h(
    'div',
    { class: 'board-toolbar' },
    h(
      'div',
      { class: 'board-tools' },
      h('input', {
        class: 'field board-search',
        type: 'search',
        'aria-label': 'Rechercher un média',
        placeholder: 'Rechercher un média',
        value: state.search,
        onInput: (event) => {
          state.search = event.target.value;
          refreshGrid(true);
        },
      }),
      h(
        'div',
        { class: 'chips', role: 'group', 'aria-label': 'Filtrer par type de média' },
        Object.entries(KIND_LABELS).map(([id, label]) =>
          h(
            'button',
            {
              class: `chip${state.kind === id ? ' is-active' : ''}`,
              'data-kind': id,
              'aria-pressed': String(state.kind === id),
              onClick: () => setKind(id),
            },
            label,
          ),
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
      h(
        'div',
        { class: 'board-actions' },
        state.items.length > 0
          ? h(
              'button',
              {
                class: 'btn is-quiet',
                'aria-pressed': String(state.selecting),
                onClick: () => (state.selecting ? leaveSelection() : enterSelection()),
              },
              state.selecting ? 'Terminer' : 'Sélectionner',
            )
          : null,
        h('button', { class: 'btn is-quiet icon-btn', title: 'Actualiser', 'aria-label': 'Actualiser', onClick: loadItems }, icon('refresh')),
        can('play') ? h('button', { class: 'btn is-quiet', title: 'Lire un texte à voix haute sur les overlays', onClick: () => openSayDialog() }, icon('sound'), 'Lire…') : null,
        // A member who cannot add but can play still opens the dialog, with its
        // "Jouer une fois" action only.
        !can('addMedia') && can('play') ? h('button', { class: 'btn is-primary', onClick: openAddDialog }, icon('play'), 'Jouer') : null,
        can('addMedia') ? h('button', { class: 'btn is-primary', onClick: openAddDialog }, icon('plus'), 'Ajouter') : null,
      ),
    ),
  );

  let content;
  if (!state.itemsLoaded && !state.itemsError) {
    content = h(
      'div',
      { class: 'grid board-skeleton', role: 'status', 'aria-busy': 'true' },
      h('span', { class: 'sr-only' }, 'Chargement du board…'),
      uploads.map(uploadCard),
      boardSkeletonCards(uploads.length > 0 ? 8 : 12),
    );
  } else if (state.itemsError && state.items.length === 0 && uploads.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Board indisponible'),
      h('p', null, state.itemsError),
      h('button', { class: 'btn is-primary', onClick: retryItems }, 'Réessayer'),
    );
  } else if (state.items.length === 0 && uploads.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Le board est vide'),
      can('addMedia') ? h('button', { class: 'btn is-primary', onClick: openAddDialog }, 'Ajouter') : null,
    );
  } else if (items.length === 0 && uploads.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Aucun média trouvé'),
      h('p', null, 'Aucun média ne correspond à cette recherche ou à ces filtres.'),
      h('button', { class: 'btn', onClick: resetBoardFilters }, 'Réinitialiser la recherche et les filtres'),
    );
  } else {
    // Provisional cards always come first, whatever the search and filter.
    content = h('div', { class: `grid${state.selecting ? ' is-selecting' : ''}`, id: 'grid' }, uploads.map(uploadCard), items.map(renderCard));
  }
  // A refresh that failed keeps the last known board: say it is stale instead
  // of replacing the cards or pretending they are up to date.
  const stale =
    state.itemsError && (state.items.length > 0 || uploads.length > 0)
      ? h(
          'div',
          { class: 'board-banner is-stale', role: 'status' },
          h('span', null, 'Impossible d’actualiser le board.'),
          h('button', { class: 'btn is-quiet', onClick: retryItems }, 'Réessayer'),
        )
      : null;
  return h(
    'div',
    null,
    toolbar,
    state.filtersOpen ? renderFilters(boardUsage) : null,
    stale,
    h('div', { class: 'board' }, content, selected ? renderDetail(selected) : null),
    state.selecting ? renderSelectionBar() : null,
  );
}

// --- selection mode: pick several media, then act on all of them ---
// Bulk actions of the selection bar; each receives the picked media in
// board order. A `label` may follow that selection (the favorite toggle
// flips between adding and removing), `unavailable` returns the tooltip
// explaining a disabled button, and `danger` draws the destructive style.
const SELECTION_ACTIONS = [
  {
    label: 'Jouer à la suite',
    air: true,
    unavailable: () => (can('play') ? null : 'Ton rôle ne permet pas de lancer des médias'),
    run: playSelection,
  },
  {
    label: (items) => favoritePlan(items).label,
    unavailable: () => (canFavorite() ? null : 'Les favoris ne sont pas disponibles sur cet appareil'),
    run: favoriteSelection,
  },
  {
    label: 'Copier les liens',
    unavailable: (items) => (originLinks(items).length > 0 ? null : 'Aucun lien d’origine dans la sélection'),
    run: copySelectionLinks,
  },
  {
    label: 'Ajouter à une autre guilde…',
    unavailable: () => (importTargets(state.guilds, state.activeGuildId).length === 0 ? 'Aucune autre guilde sur ce serveur' : null),
    run: (items) => openImportDialog(items.map((item) => item.id), leaveSelection),
  },
  {
    label: 'Supprimer…',
    danger: true,
    unavailable: (items) => (deletableIds(items).length > 0 ? null : 'Aucun média supprimable (droits)'),
    run: deleteSelection,
  },
];

// Plays the picked media one after the other, in board order, and reports
// the outcome in one summary toast. A muted guild refuses like `playItem`.
async function playSelection(items) {
  const guild = activeGuild();
  if (guild && guild.membership.muted) {
    notify('Guilde en pause : reprends-la pour jouer.', true);
    return;
  }
  let sent = 0;
  let failed = 0;
  for (const item of items) {
    const response = await sendTrigger(item);
    if (response && response.ok) {
      sent += 1;
    } else {
      failed += 1;
    }
  }
  const summary = playSummary(sent, failed);
  notify(summary.message, summary.isError);
  refreshTriggerBadges();
  setTimeout(refreshQueueAutomatically, 400);
}

// Adds the favorites missing from the picked media — or removes them all
// when every one already is a favorite — updating the cards in place.
async function favoriteSelection(items) {
  const plan = favoritePlan(items);
  for (const id of plan.ids) {
    const body = await run(() => api('PUT', `/overlay/media-board/items/${encodeURIComponent(id)}/favorite`, { favorite: plan.favorite }));
    if (!body) {
      // The error is already shown; the media left can be retried.
      break;
    }
    applyFavorite(id, plan.favorite);
  }
  if (state.filters.favorites) {
    refreshGrid(false);
  }
  // The bar's favorite button follows what is left to add or remove.
  syncSelection();
  syncDetail();
}

// Copies the origin links of the picked media, one per line.
function copySelectionLinks(items) {
  const links = originLinks(items);
  if (links.length === 0) {
    return;
  }
  copyText(links.join('\n'), copiedLinksSummary(links.length));
}

// Deletes the picked media the member may edit, after one confirmation
// listing what will go (and what cannot), then leaves the selection mode.
function deleteSelection(items) {
  const deletable = deletableIds(items);
  if (deletable.length === 0) {
    return;
  }
  const dialog = deleteDialogText(deletable.length, items.length - deletable.length);
  confirmDialog(dialog.title, dialog.text, 'Supprimer', async () => {
    let deleted = 0;
    let failed = 0;
    for (const id of deletable) {
      try {
        await api('DELETE', `/overlay/media-board/items/${encodeURIComponent(id)}`);
        deleted += 1;
      } catch {
        failed += 1;
      }
    }
    const summary = deleteSummary(deleted, failed);
    notify(summary.message, summary.isError);
    applyView(CLOSED_VIEW);
    leaveSelection();
    loadItems();
  });
}

export function enterSelection() {
  clearDrawerTimers();
  // A pending single click must not open the drawer after the mode started.
  clearTimeout(clickTimer);
  clickTimer = null;
  applyView(CLOSED_VIEW);
  state.capturing = false;
  state.selecting = true;
  state.selection = new Set();
  state.selectionAnchor = null;
  renderView();
}

export function leaveSelection() {
  if (!state.selecting && state.selection.size === 0) {
    return;
  }
  state.selecting = false;
  state.selection = new Set();
  state.selectionAnchor = null;
  if (state.tab === 'board' && state.page === 'guild') {
    renderView();
  }
}

// Picked media in board order (a filtered-out media stays picked).
function pickedItems() {
  return state.items.filter((item) => state.selection.has(item.id));
}

// The media a card menu acts on: the whole selection when the card is in it.
function menuItemIds(item) {
  return menuTargetIds({
    selecting: state.selecting,
    selection: state.selection,
    itemId: item.id,
    orderedIds: state.items.map((entry) => entry.id),
  });
}

function pickItem(id, range) {
  state.selection = range
    ? selectRange(state.selection, visibleItems().map((item) => item.id), state.selectionAnchor, id)
    : toggleId(state.selection, id);
  state.selectionAnchor = id;
  syncSelection();
}

// Ctrl+click (or Cmd+click) picks a card's media even outside the selection
// mode: the first one enters the mode, the next ones toggle the media.
function pickCard(id) {
  clearTimeout(clickTimer);
  clickTimer = null;
  if (!state.selecting) {
    enterSelection();
  }
  pickItem(id, false);
}

function pickAllVisible() {
  state.selection = new Set([...state.selection, ...visibleItems().map((item) => item.id)]);
  syncSelection();
}

// The bar's button is a toggle: it clears the selection once every visible
// media is picked, and selects them all otherwise.
function toggleAllVisible() {
  const ids = visibleItems().map((item) => item.id);
  const { selectAll } = selectAllToggle({ orderedIds: ids, selection: state.selection });
  state.selection = selectAll ? new Set([...state.selection, ...ids]) : new Set();
  state.selectionAnchor = null;
  syncSelection();
}

// Updates the cards and the bar in place (no re-render: videos keep playing).
function syncSelection() {
  for (const [id, entry] of cards) {
    entry.node.classList.toggle('is-picked', state.selection.has(id));
  }
  const bar = document.getElementById('selection-bar');
  if (bar) {
    bar.replaceWith(renderSelectionBar());
  }
}

function renderSelectionBar() {
  const count = state.selection.size;
  const items = pickedItems();
  const visibleIds = visibleItems().map((item) => item.id);
  const allToggle = selectAllToggle({ orderedIds: visibleIds, selection: state.selection });
  return h(
    'div',
    { class: 'selection-bar', id: 'selection-bar', role: 'toolbar', 'aria-label': 'Sélection' },
    // The count sits on its own line above the actions; leaving the mode stays
    // on the toolbar button that opened it, so there is only one way out.
    h(
      'div',
      { class: 'selection-head' },
      h(
        'div',
        { class: 'selection-group' },
        h(
          'span',
          { class: 'selection-count' },
          count === 0 ? 'Aucun média sélectionné' : count === 1 ? '1 média sélectionné' : `${count} médias sélectionnés`,
        ),
        h('button', { class: 'btn is-quiet', onClick: toggleAllVisible }, allToggle.label),
      ),
    ),
    h(
      'div',
      { class: 'selection-actions' },
      ...SELECTION_ACTIONS.map((action) => {
        const reason = action.unavailable(items);
        return h(
          'button',
          {
            class: `btn${action.air ? ' is-air' : action.danger ? ' is-danger' : ' is-quiet'}`,
            disabled: count === 0 || Boolean(reason),
            title: reason,
            onClick: () => action.run(items),
          },
          typeof action.label === 'function' ? action.label(items) : action.label,
        );
      }),
    ),
  );
}

function resetBoardFilters() {
  state.search = '';
  state.kind = 'all';
  state.filters = { ...DEFAULT_FILTERS };
  state.filtersOpen = false;
  renderView();
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

function renderFilters(boardUsage) {
  const filters = state.filters;
  const authors = authorsOf(state.items);
  const hasActiveFilters = activeFilterCount(filters) > 0 || filters.sort !== DEFAULT_FILTERS.sort;
  const optionSwitches = [
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
  ].filter(Boolean);
  return h(
    'div',
    { class: `filters shelf${boardUsage ? ' has-usage' : ''}`, role: 'group', 'aria-label': 'Filtres du board' },
    boardUsage ? h('div', { class: 'shelf-usage' }, h('span', { class: 'meta board-usage', title: 'Occupation du board de la guilde' }, boardUsage)) : null,
    h(
      'div',
      { class: 'shelf-controls' },
      optionSwitches.length > 0
        ? h(
            'div',
            { class: 'filter-options' },
            h('span', { class: 'filter-caption' }, 'Options'),
            h('div', { class: 'switch-row' }, optionSwitches),
          )
        : null,
      optionSwitches.length > 0 ? h('span', { class: 'filter-sep' }) : null,
      filterSelect('Ajouté par', filters.author, [['', 'Tout le monde'], ...authors.map((author) => [author.id, author.name])], (author) => setFilter({ author })),
      filterSelect('Source', filters.source, [['', 'Toutes'], ...Object.entries(SOURCE_LABELS)], (source) => setFilter({ source })),
      filterSelect('Durée', filters.duration, [['', 'Toutes'], ...Object.entries(DURATION_LABELS)], (duration) => setFilter({ duration })),
      filterSelect('Trier', filters.sort, Object.entries(SORT_LABELS), (sort) => setFilter({ sort })),
      h(
        'button',
        {
          class: 'btn is-quiet filters-reset',
          disabled: !hasActiveFilters,
          onClick: () => setFilter({ ...DEFAULT_FILTERS }),
        },
        'Réinitialiser',
      ),
    ),
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
    const field = document.querySelector('.board-search');
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
  for (const chip of document.querySelectorAll('.board-toolbar .chip')) {
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

// « Afficher les détails au survol » is a device setting (Paramètres ›
// Affichage). Read once, then kept current by `overlay://device-settings`,
// like core/sound.js does for the sound settings. Off by default: a hover
// only opens the drawer when that setting is explicitly on.
let hoverDetails = false;

// Turning the setting off closes a drawer opened by a hover and drops a peek;
// a drawer pinned by a click stays. Turning it back on needs nothing.
function applyHoverDetails(view) {
  if (!view || typeof view !== 'object') {
    return;
  }
  hoverDetails = view.hoverDetails === true;
  if (hoverDetails) {
    return;
  }
  clearDrawerTimers();
  const next = pointerLeft(currentView());
  if (!sameView(next, currentView())) {
    applyView(next);
    syncDetail();
  }
}

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
  // A single card click waits to see whether it starts a double click: an
  // explicit close cancels its pending opening.
  clearTimeout(clickTimer);
  clickTimer = null;
  applyView(CLOSED_VIEW);
  state.capturing = false;
  syncDetail();
}

// Hovering a card shows its details after a short delay, also while another
// media is pinned: the drawer then peeks at the hovered one. With the device
// setting off, a hover does nothing.
function hoverDetail(id) {
  if (!hoverDetails) {
    return;
  }
  clearDrawerTimers();
  hoverTimer = setTimeout(() => {
    hoverTimer = null;
    if (!hoverDetails) {
      return;
    }
    const next = cardHovered(currentView(), id, hoverDetails);
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

// The shell keeps the device settings: read them once (the board may start
// with details on hover off) and follow every change without a restart.
invoke('settings_get').then(applyHoverDetails).catch(() => {});
listen('overlay://device-settings', applyHoverDetails);

// The drawer floats over the view, between the header and the footer. While
// the toolbar (and the filters panel) is on screen, it starts below them so
// their buttons stay reachable; scrolled past, it climbs to the view's top.
function placeDrawer(node) {
  const view = document.getElementById('view');
  if (!view || !node.isConnected) {
    return;
  }
  const box = view.getBoundingClientRect();
  const board = node.closest('.board');
  const boardTop = board ? board.getBoundingClientRect().top : box.top;
  node.style.top = `${Math.round(Math.max(box.top + 12, boardTop))}px`;
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
if (viewNode) {
  viewNode.addEventListener('scroll', replaceDrawer, { passive: true });
}
if (viewNode && typeof ResizeObserver === 'function') {
  new ResizeObserver(replaceDrawer).observe(viewNode);
}

document.addEventListener('keydown', (event) => {
  const action = escapeAction({
    key: event.key,
    // The context menu handles its own Escape (and prevents its default).
    handled: event.defaultPrevented,
    capturing: state.capturing,
    dialogOpen: Boolean(document.getElementById('dialog-root').firstElementChild),
    drawerOpen: Boolean(state.selectedId),
    selecting: state.selecting,
  });
  if (action === 'closeDetail') {
    closeDetail();
  } else if (action === 'leaveSelection') {
    leaveSelection();
  }
});

// On the board, Ctrl+A picks every visible card (like the « Tout sélectionner
// » button) and opens the selection mode if needed; elsewhere, or in a field
// being edited, it keeps its normal effect.
document.addEventListener('keydown', (event) => {
  const boardTab = state.page === 'guild' && state.tab === 'board' && Boolean(state.activeGuildId);
  const selectAllKey = (event.ctrlKey || event.metaKey) && (event.key === 'a' || event.key === 'A');
  const action = selectAllAction({
    key: event.key,
    ctrl: event.ctrlKey,
    meta: event.metaKey,
    // The context menu and the shortcut capture handle their own keys (and
    // prevent their default).
    handled: event.defaultPrevented,
    capturing: state.capturing,
    dialogOpen: Boolean(document.getElementById('dialog-root').firstElementChild),
    editableTarget: isEditable(event.target),
    canSelectAll: selectAllKey && boardTab && visibleItems().length > 0,
  });
  if (action === 'selectAll') {
    event.preventDefault();
    if (!state.selecting) {
      enterSelection();
    }
    pickAllVisible();
  }
});

// Ctrl+F focuses the media search field, like a find shortcut. The board claims
// the key even without a search field (another tab, no guild), because WebView2
// would otherwise open its « Find on page » bar on the app window.
document.addEventListener('keydown', (event) => {
  const action = findAction({
    key: event.key,
    ctrl: event.ctrlKey,
    meta: event.metaKey,
    altKey: event.altKey,
    handled: event.defaultPrevented,
    capturing: state.capturing,
    dialogOpen: Boolean(document.getElementById('dialog-root').firstElementChild),
    searchAvailable: Boolean(document.querySelector('.board-search')),
  });
  if (action === null) {
    return;
  }
  event.preventDefault();
  if (action === 'focusSearch') {
    const field = document.querySelector('.board-search');
    field.focus();
    field.select();
  }
});

// A click outside the drawer and outside a card closes it. The drawer's own
// controls, a card (which opens or picks), an open dialog and a context menu
// keep it: the drawer may stay behind the dialog it opened, and « Afficher les
// détails » must not close the drawer it just opened. Capture phase: a control
// inside the drawer may re-render it on click, and a later check would then see
// a drawer that no longer contains the clicked element.
document.addEventListener(
  'click',
  (event) => {
    const drawer = document.querySelector('.view .detail');
    if (!drawer || !(event.target instanceof Element)) {
      return;
    }
    if (drawer.contains(event.target) || event.target.closest('.card, #dialog-root, .ctx-menu')) {
      return;
    }
    closeDetail();
  },
  true,
);

// Sends one media to the overlays and records its queue badge. Returns the
// command's response, a refusal `{ ok: false, message }` when the command
// itself failed, or `null` when it answered nothing.
async function sendTrigger(item) {
  let response;
  try {
    response = await invoke('media_board_trigger', { payload: { itemId: item.id, guildId: state.activeGuildId, trigger: 'ui' } });
  } catch (error) {
    return { ok: false, message: typeof error === 'string' ? error : error.message || 'Action impossible.' };
  }
  if (response && response.jobId && (response.outcome === 'playing' || response.outcome === 'queued')) {
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
  return response || null;
}

export async function playItem(item, control) {
  const guild = activeGuild();
  if (guild && guild.membership.muted) {
    notify('Guilde en pause : reprends-la pour jouer.', true);
    return;
  }
  await performAction(control, `play:${item.id}`, async () => {
    const response = await sendTrigger(item);
    if (!response) {
      return;
    }
    if (!response.ok) {
      throw new Error(describeError(response.reason, response.message));
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
  });
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
    // The thumbnail stands in for the video until its first frame.
    const thumbnail = thumbnailOf(item);
    const poster = thumbnail ? withVersion(thumbnail.url, thumbnail.version) : null;
    const video = h('video', { src: url, preload: 'auto', playsinline: true, poster });
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

// --- title and message edited in place ---
// The field being edited survives a re-render of the board (a refresh pushed
// by the server while typing must not lose the draft).
let inlineEdit = null;

const INLINE_FIELDS = {
  title: { label: 'le titre', max: '240', empty: 'Ajouter un titre' },
  message: { label: 'la légende', max: '500', empty: 'Ajouter une légende' },
};

async function commitInline(item, field, value) {
  inlineEdit = null;
  const before = item[field] || '';
  if (value.trim() === before.trim()) {
    syncDetail();
    return;
  }
  const body = await run(() => api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, { [field]: value }));
  if (body) {
    item[field] = body.item && field in body.item ? body.item[field] : value.trim() || null;
    loadItems({ quiet: true });
  }
  syncDetail();
}

function inlineInput(item, field) {
  const spec = INLINE_FIELDS[field];
  const isTitle = field === 'title';
  const input = h(isTitle ? 'input' : 'textarea', {
    class: `field inline-input is-${field}`,
    maxlength: spec.max,
    rows: isTitle ? null : '3',
    'aria-label': `Modifier ${spec.label}`,
    value: inlineEdit.value,
  });
  let settled = false;
  const settle = (save) => {
    if (settled) {
      return;
    }
    settled = true;
    if (save) {
      commitInline(item, field, input.value);
    } else {
      inlineEdit = null;
      syncDetail();
    }
  };
  input.addEventListener('input', () => {
    inlineEdit.value = input.value;
  });
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      // Handled here: the drawer stays open.
      event.preventDefault();
      settle(false);
    } else if (event.key === 'Enter' && (isTitle || event.ctrlKey) && !event.isComposing) {
      event.preventDefault();
      settle(true);
    }
  });
  // A re-render removes the field without the user leaving it: only a real
  // blur (the field still in the page) saves.
  input.addEventListener('blur', () => {
    setTimeout(() => {
      if (input.isConnected) {
        settle(true);
      }
    });
  });
  queueMicrotask(() => {
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  });
  return input;
}

function inlineField(item, field) {
  const spec = INLINE_FIELDS[field];
  const isTitle = field === 'title';
  const text = isTitle ? itemTitle(item) : item.message || '';
  if (!item.canEdit) {
    if (isTitle) {
      return h('h2', { class: 'detail-title' }, text);
    }
    return text ? h('p', { class: 'detail-message' }, text) : null;
  }
  if (inlineEdit && inlineEdit.itemId === item.id && inlineEdit.field === field) {
    return inlineInput(item, field);
  }
  return h(
    'button',
    {
      class: `inline-edit is-${field}${text ? '' : ' is-empty'}`,
      type: 'button',
      title: `Modifier ${spec.label}`,
      onClick: () => {
        inlineEdit = { itemId: item.id, field, value: item[field] || '' };
        syncDetail();
      },
    },
    h('span', { class: `inline-text${isTitle ? ' is-title' : ''}` }, text || spec.empty),
    icon('edit'),
  );
}

function renderDetail(item) {
  const preview = detailPreview(item);
  if (inlineEdit && inlineEdit.itemId !== item.id) {
    inlineEdit = null;
  }
  const shortcut = shortcutFor(item.id);
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
      { class: 'detail-body' },
      h(
        'div',
        { class: 'detail-head' },
        inlineField(item, 'title'),
        canFavorite()
          ? h(
              'button',
              {
                class: `btn is-quiet fav-toggle${item.favorite ? ' is-on' : ''}`,
                'aria-pressed': String(Boolean(item.favorite)),
                title: item.favorite ? 'Retirer des favoris' : 'Ajouter aux favoris',
                onClick: (event) => toggleFavorite(item, event.currentTarget),
              },
              icon('star'),
            )
          : null,
        h('button', { class: 'btn is-quiet icon-btn', 'aria-label': 'Fermer les détails', onClick: closeDetail }, icon('close')),
      ),
      preview,
      h(
        'dl',
        { class: 'facts' },
        // The caption shown on the overlays, then the shortcut that plays it.
        item.canEdit || item.message
          ? [h('dt', null, 'Légende'), h('dd', null, inlineField(item, 'message'))]
          : null,
        can('play')
          ? [
              h('dt', null, 'Raccourci'),
              h('dd', { class: 'shortcut-cell' }, shortcutControl(item, shortcut)),
            ]
          : null,
      ),
      can('play') ? h('p', { class: 'shortcut-hint' }, ALTGR_HINT) : null,
    ),
    h(
      'div',
      { class: 'detail-foot' },
      can('play') ? h('button', { class: 'btn is-air is-wide', onClick: (event) => playItem(item, event.currentTarget) }, icon('play'), 'Jouer sur les overlays') : null,
      h(
        'div',
        { class: 'detail-actions' },
        canOpenEditor(item) ? h('button', { class: 'btn is-quiet', title: 'Découper, recadrer, dessiner…', onClick: () => openMediaEditor(item, itemTitle(item)) }, icon('edit'), 'Modifier') : null,
        h('button', { class: 'btn is-quiet', onClick: () => openInfoDialog(item) }, 'Informations'),
        item.canEdit ? h('button', { class: 'btn is-quiet is-danger', onClick: () => openDeleteDialog(item) }, 'Supprimer…') : null,
      ),
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

function cancelCapture() {
  state.capturing = false;
  renderView();
}

// The shortcut value is itself the button that records a new one; a small
// cross next to it clears it.
function shortcutControl(item, shortcut) {
  const capturing = state.capturing && captureItem && captureItem.id === item.id;
  if (capturing) {
    return h(
      'button',
      {
        class: 'shortcut-key is-capturing',
        type: 'button',
        title: 'Annuler',
        onClick: cancelCapture,
      },
      'Appuie sur la combinaison…',
      h('kbd', null, 'Échap'),
    );
  }
  if (!shortcut) {
    return h(
      'button',
      { class: 'shortcut-key is-empty', type: 'button', onClick: () => startCapture(item) },
      icon('plus'),
      'Définir',
    );
  }
  return [
    h(
      'button',
      { class: 'shortcut-key', type: 'button', title: 'Changer le raccourci', onClick: () => startCapture(item) },
      h('kbd', null, shortcutLabel(shortcut)),
      icon('edit'),
    ),
    h(
      'button',
      {
        class: 'btn is-quiet shortcut-clear',
        type: 'button',
        title: 'Retirer le raccourci',
        'aria-label': 'Retirer le raccourci',
        onClick: () => saveShortcut(item, null),
      },
      icon('close'),
    ),
  ];
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
  if (accelerator === UNSUPPORTED_KEY) {
    notify(UNSUPPORTED_KEY_MESSAGE, true);
    return;
  }
  if (accelerator === undefined) {
    notify(BARE_KEY_MESSAGE, true);
    return;
  }
  const keyLabel = capturedKeyLabel(event);
  state.capturing = false;
  saveShortcut(captureItem, accelerator, keyLabel);
});

export async function saveShortcut(item, accelerator, keyLabel = null) {
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
    const failed = Array.isArray(response.failedAccelerators) && response.failedAccelerators.includes(accelerator);
    if (accelerator && !failed && state.bindings[accelerator] === item.id) {
      if (keyLabel) {
        writeShortcutLabel(accelerator, keyLabel);
      } else {
        forgetShortcutLabel(accelerator);
      }
    }
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

// Length shown for an item: the media length, or for an image the display
// time chosen for it (none when it uses the guild default).
function shownDuration(item) {
  return item.media.kind === 'image' ? item.displayDurationSec ?? null : item.media.durationSec;
}

// Placeholder of the display time field: the guild default when known.
function defaultDurationHint() {
  const settings = state.info && state.info.guild && state.info.guild.settings;
  const seconds = settings && settings.defaultMediaTime;
  return Number.isFinite(seconds) && seconds > 0 ? `Par défaut (${seconds} s)` : 'Par défaut';
}

function infoFacts(item) {
  const link = sourceLink(item);
  const size = item.media.sizeBytes ? `${(item.media.sizeBytes / 1048576).toFixed(1).replace('.', ',')} Mo` : null;
  const added = new Date(item.createdAt);
  return h(
    'dl',
    { class: 'facts' },
    [h('dt', null, 'Ajouté par'), h('dd', null, item.createdByName || 'Inconnu')],
    [h('dt', null, 'Le'), h('dd', null, Number.isNaN(added.getTime()) ? '—' : added.toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: 'numeric' }))],
    link
      ? [
          h('dt', null, 'Source'),
          h(
            'dd',
            { class: 'source-cell' },
            h('span', { class: 'source-url' }, link),
            h(
              'button',
              {
                class: 'btn is-quiet icon-btn',
                type: 'button',
                title: 'Copier le lien d’origine',
                'aria-label': 'Copier le lien d’origine',
                onClick: () => copyText(link, 'Lien copié.'),
              },
              icon('copy'),
            ),
          ),
        ]
      : null,
    [h('dt', null, 'Format'), h('dd', null, [KIND_LABELS[item.media.kind] || item.media.kind, formatDuration(shownDuration(item)), size, item.displayScalePercent ? `affiché à ${item.displayScalePercent} %` : null].filter(Boolean).join(', '))],
  );
}

// The media's secondary facts. A media the user can edit also gets here what
// is not edited in the drawer: an audio's picture, an image's display time.
export function openInfoDialog(item) {
  const isAudio = item.media.kind === 'audio';
  const isImage = item.media.kind === 'image';
  if (!item.canEdit || !(isAudio || isImage)) {
    openDialog((close) => [
      h('h2', null, itemTitle(item)),
      infoFacts(item),
      h('div', { class: 'actions' }, h('button', { class: 'btn', type: 'button', onClick: close }, 'Fermer')),
    ]);
    return;
  }
  openDialog((close) => {
    const displayDuration = isImage
      ? h('input', {
          class: 'field',
          type: 'number',
          min: '1',
          max: String(MAX_DISPLAY_DURATION_SEC),
          step: '1',
          inputmode: 'numeric',
          value: item.displayDurationSec ?? '',
          placeholder: defaultDurationHint(),
          style: 'max-width:160px',
        })
      : null;
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
          const patch = {};
          if (isAudio && onOverlay.checked !== Boolean(item.artworkOnOverlay)) {
            patch.artworkOnOverlay = onOverlay.checked;
          }
          if (isImage) {
            const parsed = parseDisplayDuration(displayDuration.value);
            if (!parsed.ok) {
              notify(INVALID_DURATION_MESSAGE, true);
              return;
            }
            if (parsed.value !== (item.displayDurationSec ?? null)) {
              patch.displayDurationSec = parsed.value;
            }
          }
          if (Object.keys(patch).length > 0) {
            const body = await run(() => api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, patch));
            if (!body) {
              return;
            }
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
      displayDuration ? h('label', null, 'Durée d’affichage (s)', displayDuration) : null,
      artworkFields,
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, itemTitle(item)), infoFacts(item), form];
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

// The window behaves like an application: a drag started inside it (a
// thumbnail, a link, a field's selected text) never leaves as a native ghost
// copy. Queue rows are the one deliberate native drag: queue.js reorders them.
// Files and links dropped from outside do not fire dragstart here: only they
// are still imports.
document.addEventListener('dragstart', (event) => {
  if (!event.target.closest?.('.line.is-queue')) {
    event.preventDefault();
  }
});

function transfersFiles(event) {
  const transfer = event.dataTransfer;
  if (!transfer) {
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

function boardIsActive() {
  return Boolean(state.activeGuildId) && state.page === 'guild' && state.tab === 'board';
}

function canDropFiles() {
  return boardIsActive() && can('addMedia');
}

// A paste opens the dialog, which also offers "Jouer une fois": playback is
// enough, unlike a drop, which uploads straight to the board.
function canPasteFiles() {
  return boardIsActive() && (can('addMedia') || can('play'));
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

// --- pasting media on the board ---

// A media copied in the Windows file explorer (or an image copied from a page)
// arrives as a file: Ctrl+V opens the add-media dialog with it, like a link
// dropped on the board opens it with the link. Text keeps its native paste
// meaning, and a field or another dialog keeps its own paste.
document.addEventListener('paste', (event) => {
  if (event.defaultPrevented || isEditable(event.target)) {
    return;
  }
  const dialogRoot = document.getElementById('dialog-root');
  if (dialogRoot && dialogRoot.firstElementChild) {
    return;
  }
  if (!canPasteFiles()) {
    return;
  }
  const files = pastedMediaFiles(event.clipboardData);
  if (files.length === 0) {
    return;
  }
  event.preventDefault();
  openAddDialog({ files });
});

document.addEventListener('dragend', resetDropVeil);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && dropVeil && !dropVeil.hidden) {
    dropDismissed = true;
    hideDropVeil();
  }
});
