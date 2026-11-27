// Media board: grid, cards, detail window, play, shortcuts, edit and delete.

import { api, run } from '../core/api.js';
import { performAction } from '../core/action-ui.js';
import { afterNextClick } from '../core/after-click.js';
import { contextMenuOpen, isEditable, openContextMenu } from '../core/contextmenu.js';
import { findAction } from '../core/find-shortcut.js';
import { acceleratorFromEvent, BARE_KEY_MESSAGE, UNSUPPORTED_KEY, UNSUPPORTED_KEY_MESSAGE, cancelsCapture } from '../core/accelerators.js';
import { ARTWORK_MESSAGES, artworkExtension, droppedArtworkLink, fetchArtwork, MAX_ARTWORK_BYTES, pickArtworkFile } from '../core/artwork.js';
import { backToTopBehavior, showsBackToTop } from '../core/back-to-top.js';
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
import { syncKindChips } from '../core/kind-chips.js';
import {
  artworkOf,
  cardPicture,
  cardPreviewAt,
  mediaPath,
  pictureKey,
  realignPreviewAt,
  thumbnailOf,
  withVersion,
} from '../core/detail-drawer.js';
import { detailActionIds, DOUBLE_CLICK_WINDOW, previewAllowed, previewMeta } from '../core/detail-actions.js';
import { h, icon, prefersReducedMotion, registerIcons } from '../core/dom.js';
import { formatBoardUsage, formatDuration, INVALID_DURATION_MESSAGE, MAX_DISPLAY_DURATION_SEC, parseDisplayDuration, relativeDateLabel } from '../core/format.js';
import { DROP_VEIL_TEXT, dropTarget } from '../core/media-drop.js';
import { pastedMediaFiles } from '../core/media-files.js';
import { badgeForItem } from '../core/queue-badges.js';
import {
  cardPointerAction,
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
import { mediaLevel } from '../core/sound.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText } from '../core/ui.js';
import { loadItems, refreshQueueAutomatically } from '../data.js';
import { addDialogOpen, dropOnAddDialog, highlightAddDialog, openAddDialog } from './add-media.js';
import { openImportDialog } from './import-media.js';
import { createMediaDetail } from './media-detail.js';
import { openSayDialog } from './say.js';
import { canOpenEditor, openMediaEditor } from './editor/index.js';
import { openUploadMenu, renderUploadCard, startFileUpload, startLinkRefresh, uploadsForGuild } from './uploads.js';
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
  menuItemId = item.id;
  const link = sourceLink(item);
  openContextMenu(event, [
    { heading: itemTitle(item) },
    can('play') ? { label: 'Jouer sur les overlays', icon: 'play', onSelect: () => playItem(item) } : null,
    {
      label: 'Détails',
      onSelect: () => selectItem(item.id),
    },
    canFavorite()
      ? { label: item.favorite ? 'Retirer' : 'Favori', icon: 'star', onSelect: () => toggleFavorite(item) }
      : null,
    canOpenEditor(item) ? { label: 'Modifier', icon: 'edit', onSelect: () => openMediaEditor(item, itemTitle(item)) } : null,
    can('play')
      ? {
          label: shortcutFor(item.id) ? 'Changer le raccourci' : 'Définir un raccourci',
          onSelect: () => {
            // The capture lives in the window's information column: open it
            // first, then start listening.
            selectItem(item.id);
            state.capturing = false;
            startCapture(item);
          },
        }
      : null,
    link ? { label: 'Copier le lien d’origine', icon: 'copy', onSelect: () => copyText(link, 'Lien copié.') } : null,
    link && can('addMedia') && item.media.edited !== true
      ? { label: 'Rafraîchir depuis la source', icon: 'refresh', onSelect: () => refreshSource(item) }
      : null,
    importTargets(state.guilds, state.activeGuildId).length > 0
      ? { label: 'Ajouter à une autre guilde…', onSelect: () => openImportDialog(menuItemIds(item), leaveSelection) }
      : null,
    item.canEdit ? 'separator' : null,
    item.canEdit ? { label: 'Supprimer…', icon: 'remove', danger: true, onSelect: () => openDeleteDialog(item) } : null,
  ]);
}

// C: re-download a card's media from its source. The server updates the asset
// in place and answers the same item, so the card stays where it is. An edited
// card is content-addressed: forcing its URL would create the original as a
// second card, not refresh the edit, so the action is not offered for it.
function refreshSource(item) {
  if (!startLinkRefresh({ guildId: state.activeGuildId, item })) {
    notify('Ce média ne peut pas être rafraîchi.', true);
  }
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
  } else {
    syncDetail();
  }
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

// One meta line under the title: the kind (icon and label), the author and the
// relative date, separated by dots. Everything is one no-wrap line; the author
// is the part that gives way when the card is narrow.
function cardMeta(item) {
  const kind = item.media.kind;
  const author = authorName(item);
  const date = relativeDateLabel(item.createdAt);
  return h(
    'p',
    { class: 'card-meta' },
    icon(kind),
    h('span', { class: 'card-meta-kind' }, KIND_LABELS[kind] || kind),
    h('span', { class: 'card-meta-dot' }, '·'),
    h('span', { class: 'card-meta-author', title: author }, author),
    date ? h('span', { class: 'card-meta-dot' }, '·') : null,
    date ? h('span', { class: 'card-meta-date' }, date) : null,
  );
}

// The card's Play button, when the member may play: a double click plays
// through it so the card shows the same busy state as the button's own click.
function cardPlayButton(node) {
  return node.closest('.card')?.querySelector('.play') || null;
}

// A single click opens the details at once. The second click of a double click
// then lands on the window the first click just opened, so the board catches it
// there (same place, one double-click window) before the window acts: that click
// closes the window and plays the media. Nothing waits between the two clicks.
let armedCardClick = null;
let swallowCardDoubleClick = 0;

// A small tolerance around the armed click: a click meant for the window itself
// lands farther away, one on the same spot is the second click of the double click.
const CARD_DOUBLE_CLICK_SLOP = 12;

function armCardDoubleClick(entry, event) {
  // Without the Play command the second click has nothing to launch: it keeps
  // acting on the window, as a double click did when it only opened the details.
  if (!can('play') || state.selecting || state.capturing) {
    return;
  }
  armedCardClick = {
    entry,
    control: cardPlayButton(event.currentTarget),
    x: event.clientX,
    y: event.clientY,
    at: performance.now(),
  };
}

function catchCardDoubleClick(event) {
  const armed = armedCardClick;
  if (!armed) {
    return;
  }
  // The next click says what the armed one was: its double click, or a new
  // click that belongs to the window.
  armedCardClick = null;
  if (event.detail < 2) {
    return;
  }
  if (
    performance.now() - armed.at > DOUBLE_CLICK_WINDOW ||
    Math.hypot(event.clientX - armed.x, event.clientY - armed.y) > CARD_DOUBLE_CLICK_SLOP ||
    !can('play') ||
    !document.querySelector('.hp-page-shell')?.contains(event.target) ||
    mediaDetail.openId() !== armed.entry.item.id
  ) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  swallowCardDoubleClick = performance.now();
  closeDetail();
  playItem(armed.entry.item, armed.control);
}

// The native double click that follows the second click would reach the details
// window (its body reopens the window): swallow it after the play.
function swallowCardDoubleClickEvent(event) {
  const at = swallowCardDoubleClick;
  if (at === 0) {
    return;
  }
  swallowCardDoubleClick = 0;
  if (event.detail < 2 || performance.now() - at > DOUBLE_CLICK_WINDOW) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();
}

// Runs what cardPointerAction decided for a card: opening the window, playing
// the media, and the selection gestures.
function applyCardPointerAction(action, entry, control = null) {
  if (action === 'details') {
    selectItem(entry.item.id);
  } else if (action === 'play') {
    if (can('play')) {
      // The first click opened this card's window; the second click plays the
      // media instead, so the window it opened goes away with it.
      if (mediaDetail.openId() === entry.item.id) {
        closeDetail();
      }
      playItem(entry.item, control);
    } else {
      // Without the Play command, a double click keeps opening the details.
      selectItem(entry.item.id);
    }
  } else if (action === 'pick') {
    pickCard(entry.item.id);
  } else if (action === 'toggle' || action === 'range') {
    pickItem(entry.item.id, action === 'range');
  }
}

// One handler for the thumbnail and the card's background: they decide the same
// click, and a single click arms the catch of its possible double click.
function handleCardClick(entry, event, onControl = false) {
  const action = cardPointerAction({
    selecting: state.selecting,
    clickCount: event.detail,
    shiftKey: event.shiftKey,
    ctrlKey: event.ctrlKey,
    metaKey: event.metaKey,
    onControl,
    hoverPreview,
  });
  applyCardPointerAction(action, entry, cardPlayButton(event.currentTarget));
  if (action === 'details' && event.detail === 1 && !hoverPreview) {
    armCardDoubleClick(entry, event);
  }
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
      onClick: (event) => handleCardClick(entry, event),
    },
    h('span', { class: 'glyph' }, icon(kind === 'audio' ? 'audio' : kind === 'image' ? 'image' : 'video')),
    h('span', { class: 'pick', 'aria-hidden': 'true' }, icon('check')),
    artwork ? h('span', { class: 'kind-badge', 'aria-hidden': 'true' }, icon('audio')) : null,
    formatDuration(shownDuration(item)) ? h('span', { class: 'duration' }, formatDuration(shownDuration(item))) : null,
  );
  if (picture.url) {
    thumbObserver.observe(thumb);
  }
  const node = h(
    'div',
    {
      class: 'card',
      'data-item-id': item.id,
      onContextmenu: (event) => openCardMenu(event, entry.item),
      // The thumbnail owns its own clicks; the card's background follows the
      // same click decision (details, play or selection).
      onClick: (event) => {
        if (event.target.closest('.thumb')) {
          return;
        }
        // The card's buttons (play, favorite) keep their own click.
        handleCardClick(entry, event, Boolean(event.target.closest('button, input, a, select')));
      },
      onMouseenter: (event) => {
        mediaDetail.prefetchCard(event.currentTarget);
        mediaDetail.hoverCard(event.currentTarget);
      },
      onMouseleave: (event) => {
        mediaDetail.cancelPrefetch(event.currentTarget);
        mediaDetail.leaveCard(event.currentTarget);
      },
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
    h(
      'div',
      { class: 'card-body' },
      h('p', { class: 'card-title', title: itemTitle(item) }, itemTitle(item)),
      cardMeta(item),
    ),
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

// Every card of the regular grid has the same shape: a fixed run of
// placeholders is enough to paint the first screenful, whatever the media the
// guild holds. Only the first screenful is painted, so the shimmer never
// animates dozens of off-screen gradients.
const SKELETON_CARDS = 12;

// First load: the grid's shape is painted immediately, so the board no longer
// looks like an empty guild while the first request is in flight.
function boardSkeletonCards(count = SKELETON_CARDS) {
  return Array.from({ length: count }, () => {
    return h(
      'div',
      { class: 'card is-skeleton', 'aria-hidden': 'true' },
      h('div', { class: 'card-frame' }, h('span', { class: 'skeleton-block' })),
      h(
        'div',
        { class: 'card-body' },
        h(
          'div',
          { class: 'card-title-skeleton' },
          h('span', { class: 'skeleton-block' }),
          h('span', { class: 'skeleton-block is-short' }),
        ),
        h('div', { class: 'card-meta-skeleton' }, h('span', { class: 'skeleton-block' })),
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
    closeDetail();
    return h('div', { class: 'empty' }, h('h2', null, 'Board réservé aux membres'), h('p', null, "Cette guilde ne montre pas son board aux invités. Tu reçois quand même les médias diffusés."));
  }
  pruneCards();
  const items = visibleItems();
  const uploads = activeUploads();
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
                id: 'select-toggle',
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
  // The tools and the open filters panel share one sticky head (`.board-head`):
  // they stay on screen while the cards scroll behind its background. Their
  // margins are painted by that background too, so no card shows through.
  const head = h(
    'div',
    { class: 'board-head' },
    toolbar,
    state.filtersOpen ? renderFilters(boardUsage) : null,
  );

  let content;
  if (!state.itemsLoaded && !state.itemsError) {
    content = h(
      'div',
      { class: 'grid board-skeleton', role: 'status', 'aria-busy': 'true' },
      h('span', { class: 'sr-only' }, 'Chargement du board…'),
      uploads.map(uploadCard),
      boardSkeletonCards(uploads.length > 0 ? 8 : SKELETON_CARDS),
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
    content = h(
      'div',
      { class: `grid${state.selecting ? ' is-selecting' : ''}${hoverPreview ? ' is-hover-preview' : ''}`, id: 'grid' },
      uploads.map(uploadCard),
      items.map(renderCard),
    );
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
  // The detail window lives outside the view: a board refresh only rebuilds
  // its information column, and closes it when its media left the board.
  syncDetail();
  return h(
    'div',
    null,
    head,
    stale,
    h('div', { class: 'board' }, content),
    state.selecting ? renderSelectionBar() : null,
    // Last, so `.board + .selection-bar` stays adjacent for the bar's bottom
    // padding; the fixed button takes no slot in the flow.
    backToTopButton(),
  );
}

// --- retour en haut ---
// The board is the only view that scrolls: once the toolbar has left with the
// first rows, this floating button brings the grid back. It belongs to the
// board's tree, so the other tabs (queue, settings, admin) never show it.
function backToTopButton() {
  const view = document.getElementById('view');
  // The router mounts this tree, then puts the remembered scroll back: recheck
  // once that is done, so a tab switch from a scrolled page never flashes the
  // button over a board that is back at its top.
  queueMicrotask(syncBackToTop);
  return h(
    'button',
    {
      class: 'back-to-top',
      type: 'button',
      title: 'Revenir en haut',
      'aria-label': 'Revenir en haut',
      hidden: !view || !showsBackToTop(view.scrollTop),
      onClick: () => {
        const node = document.getElementById('view');
        if (node) {
          node.scrollTo({ top: 0, behavior: backToTopBehavior(prefersReducedMotion()) });
        }
      },
    },
    icon('up'),
  );
}

// Each render builds a new button: this keeps the one on screen in step with
// the view's scroll, whichever render put it there. Other views have no button
// and are left alone.
function syncBackToTop() {
  const view = document.getElementById('view');
  const button = view ? view.querySelector('.back-to-top') : null;
  if (button) {
    button.hidden = !showsBackToTop(view.scrollTop);
  }
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
    notify('Guilde coupée ici : réactive-la pour jouer.', true);
    return;
  }
  let sent = 0;
  let failed = 0;
  for (const item of items) {
    const caption = legendDraft(item.id);
    const response = await sendTrigger(item, caption);
    if (response && response.ok) {
      sent += 1;
      if (legendDraft(item.id) === caption) {
        legendDrafts.delete(item.id);
      }
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
    closeDetail();
    leaveSelection();
    loadItems();
  });
}

export function enterSelection() {
  // The selection takes over the board: the window and the preview go.
  closeDetail();
  mediaDetail.closePreview();
  state.capturing = false;
  state.selecting = true;
  state.selection = new Set();
  state.selectionAnchor = null;
  // Paints the mode in place: the grid is not rebuilt, so the scroll position
  // the viewer was reading cannot move. Rebuilding the whole view here is what
  // sent the board back to the top when the mode started.
  syncSelection();
  syncSelectionToggle();
  document.getElementById('grid')?.classList.add('is-selecting');
}

export function leaveSelection() {
  if (!state.selecting && state.selection.size === 0) {
    return;
  }
  state.selecting = false;
  state.selection = new Set();
  state.selectionAnchor = null;
  // Same in-place swap as entering: the cards stay, only the mode chrome goes.
  syncSelection();
  document.getElementById('grid')?.classList.remove('is-selecting');
  syncSelectionToggle();
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

// Updates the cards and the bar in place (no re-render: videos keep playing
// and the scroll position survives).
function syncSelection() {
  for (const [id, entry] of cards) {
    entry.node.classList.toggle('is-picked', state.selection.has(id));
  }
  syncSelectionBar();
}

// The bar is created when the mode starts, then replaced in place on every
// pick; the toolbar above it and the grid are left alone.
function syncSelectionBar() {
  const bar = document.getElementById('selection-bar');
  if (!state.selecting) {
    bar?.remove();
    return;
  }
  const next = renderSelectionBar();
  if (bar) {
    bar.replaceWith(next);
  } else {
    document.querySelector('.view .board')?.after(next);
  }
}

// The toolbar button flips between « Sélectionner » and « Terminer » in place:
// rebuilding the toolbar would take the search field and the scroll with it.
function syncSelectionToggle() {
  const toggle = document.getElementById('select-toggle');
  if (!toggle) {
    return;
  }
  toggle.setAttribute('aria-pressed', String(state.selecting));
  toggle.textContent = state.selecting ? 'Terminer' : 'Sélectionner';
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
  // A filter can hide the media the window shows: it then closes instead of
  // staying over a card that is no longer on the board.
  syncDetail();
}

function setKind(kind) {
  state.kind = kind;
  syncKindChips(document.querySelectorAll('.board-toolbar .chip'), kind);
  refreshGrid(false);
}

// --- detail window ---------------------------------------------------------
// « Ouvrir l'aperçu au survol » is a device setting (Paramètres › Affichage).
// Read once, then kept current by `overlay://device-settings`, like
// core/sound.js does for the sound settings. On by default: the shell may turn
// it off, and the value stays on until it says otherwise.
let hoverPreview = true;

// The card whose menu is open, if any. The preview stays off that one card —
// it covers its card whole and would hide the menu it just opened — while the
// pointer moving on to another card still opens that card's preview.
let menuItemId = null;

// The caption is a draft owned by the detail window: typed there, sent with
// the next trigger and never stored on the server. A fresh opening starts
// empty even when the item still carries an old server-side message, and both
// a send and the window's close drop the draft.
const legendDrafts = new Map();

function legendDraft(itemId) {
  return (legendDrafts.get(itemId) || '').trim();
}

// One controller for the board, created once: the hover preview, the modal
// detail window and their transitions live in views/media-detail.js.
const mediaDetail = createMediaDetail({
  grid: () => document.getElementById('grid'),
  view: () => document.getElementById('view'),
  hoverEnabled: (itemId) => previewAllowed({
    hoverPreview,
    selecting: state.selecting,
    capturing: state.capturing,
    dialogOpen: Boolean(document.getElementById('dialog-root')?.firstElementChild),
    menuOnCard: contextMenuOpen() && itemId === menuItemId,
  }),
  describe: (id) => describeItem(id),
  // The right click lands on the preview that covers its card: it opens the
  // card's own menu, so both paths keep one definition.
  onCardMenu: (event, card) => {
    const entry = cards.get(card.dataset.itemId);
    if (entry) {
      openCardMenu(event, entry.item);
    }
  },
  onBeforeOpen: (id) => {
    // The draft belongs to one opening: a media reopened later never brings
    // back what was typed for a previous window (nor its old stored message).
    legendDrafts.delete(id);
  },
  onOpen: (id) => {
    state.detailId = id;
  },
  onClose: (id) => {
    // A newer window may already be open while the old one fades out: its id,
    // its caption draft and its capture must survive this late close.
    if (mediaDetail.openId() !== id) {
      legendDrafts.delete(id);
    }
    if (mediaDetail.openId()) {
      return;
    }
    // A shortcut capture lives in the window's control: closing the window
    // (backdrop, back button, Escape) stops listening for a key.
    state.capturing = false;
    state.detailId = null;
  },
});

// Turning the setting off closes the preview at once. Turning it back on
// needs nothing; a window already open is not a hover and stays.
function applyHoverPreview(view) {
  if (!view || typeof view !== 'object') {
    return;
  }
  hoverPreview = view.hoverPreview !== false;
  // The card's own hover (lift, zoom, buttons) only plays without the preview.
  document.getElementById('grid')?.classList.toggle('is-hover-preview', hoverPreview);
  if (!hoverPreview) {
    mediaDetail.closePreview();
  }
}

// The shell keeps the device settings: read them once (the board's preview is
// on until the shell says otherwise) and follow every change without a restart.
invoke('settings_get').then(applyHoverPreview).catch(() => {});
listen('overlay://device-settings', applyHoverPreview);

// Opens the detail window at once. Its callers ask on a single click (with the
// hover preview off), a keyboard click, the context menu or the shortcut
// capture; the second click of a double click is caught on the open window.
export function selectItem(id) {
  if (!cards.has(id)) {
    return;
  }
  mediaDetail.openDetail(cards.get(id).node);
}

// The window survives a board render (it lives outside the view): a refresh
// rebuilds only its information column. A media deleted or filtered out of the
// visible board closes it (the module then fades out instead of flying back).
function syncDetail() {
  const id = mediaDetail.openId();
  if (!id) {
    return;
  }
  if (!visibleItems().some((item) => item.id === id)) {
    closeDetail();
    return;
  }
  mediaDetail.refreshDetail();
}

export function closeDetail() {
  // Closing the window while it was recording a shortcut cancels the capture:
  // the control that listens for it is gone.
  state.capturing = false;
  mediaDetail.closeDetail();
}

// The second click of a double click is caught before the details window sees
// it (capture), and the native double click that follows is swallowed with it.
document.addEventListener('click', catchCardDoubleClick, true);
document.addEventListener('dblclick', swallowCardDoubleClickEvent, true);

// The board is the only view that scrolls `#view`: the floating back-to-top
// follows that scroller. The view router swaps the view's children, not the
// view itself, so one listener lasts for the whole session.
const viewNode = document.getElementById('view');
if (viewNode) {
  viewNode.addEventListener('scroll', syncBackToTop, { passive: true });
}

document.addEventListener('keydown', (event) => {
  const action = escapeAction({
    key: event.key,
    // The context menu handles its own Escape (and prevents its default).
    handled: event.defaultPrevented,
    capturing: state.capturing,
    dialogOpen: Boolean(document.getElementById('dialog-root').firstElementChild),
    detailOpen: Boolean(mediaDetail.openId()),
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

// Sends one media to the overlays and records its queue badge. Returns the
// command's response, a refusal `{ ok: false, message }` when the command
// itself failed, or `null` when it answered nothing.
async function sendTrigger(item, message = legendDraft(item.id)) {
  let response;
  try {
    response = await invoke('media_board_trigger', { payload: { itemId: item.id, guildId: state.activeGuildId, trigger: 'ui', message } });
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
    notify('Guilde coupée ici : réactive-la pour jouer.', true);
    return;
  }
  const caption = legendDraft(item.id);
  const sent = await performAction(control, `play:${item.id}`, async () => {
    const response = await sendTrigger(item, caption);
    if (!response) {
      return false;
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
    return true;
  });
  // A sent caption is consumed once: the next trigger starts without it. A
  // newer draft typed while the request was in flight is kept.
  if (sent && legendDraft(item.id) === caption) {
    legendDrafts.delete(item.id);
    syncDetail();
  }
}

export function shortcutFor(itemId) {
  const entry = Object.entries(state.bindings).find(([, id]) => id === itemId);
  return entry ? entry[0] : null;
}

// --- detail window content -------------------------------------------------
// A loaded thumbnail URL is reused while it stays valid long enough to watch
// the media: the browser already has its first bytes.
const PREVIEW_URL_MARGIN_MS = 10 * 60 * 1000;

function previewUrl(item) {
  const card = cards.get(item.id);
  const loaded = card && card.loadedUrl;
  if (loaded && mediaPath(loaded) === mediaPath(item.mediaUrl) && urlExpiresAt(loaded) - Date.now() > PREVIEW_URL_MARGIN_MS) {
    return loaded;
  }
  return item.mediaUrl || null;
}

// Everything views/media-detail.js needs to know about one media. The module
// only handles ids: the board state and the cards stay here.
function describeItem(id) {
  const item = state.items.find((entry) => entry.id === id);
  if (!item) {
    return null;
  }
  const kind = item.media.kind;
  const artwork = kind === 'audio' ? artworkOf(item) : null;
  const thumbnail = thumbnailOf(item);
  const picture = artwork || thumbnail;
  const poster = picture ? withVersion(picture.url, picture.version) : '';
  // The preview box takes the media's own shape; an audio takes its picture's.
  const shape = artwork || item.media;
  const width = Number(shape.width);
  const height = Number(shape.height);
  const url = previewUrl(item);
  return {
    kind,
    url,
    expiresAt: urlExpiresAt(url),
    poster,
    ratio: width > 0 && height > 0 ? width / height : null,
    volume: mediaLevel(state.activeGuildId),
    label: itemTitle(item),
    previewBody: () => previewBody(item),
    detailBody: () => detailBody(item),
    // The top bar's tools: the favorite toggle, at the right of the window's
    // top bar (see describe(id).tools() in media-detail.js).
    tools: () => (canFavorite() ? favoriteTool(item) : null),
  };
}

// The favorite toggle that sits in the window's top bar. It is rebuilt with
// the rest of the window on every board refresh, so it always shows the
// item's current state.
function favoriteTool(item) {
  const favorite = Boolean(item.favorite);
  return h(
    'button',
    {
      type: 'button',
      class: `btn is-quiet fav-toggle hp-fav${favorite ? ' is-on' : ''}`,
      'aria-pressed': String(favorite),
      title: favorite ? 'Retirer des favoris' : 'Ajouter aux favoris',
      onClick: (event) => toggleFavorite(item, event.currentTarget),
    },
    h('span', { class: 'hp-fav-glyph' }, icon('star')),
    h('span', { class: 'hp-fav-label' }, favorite ? 'Retirer des favoris' : 'Ajouter aux favoris'),
  );
}

// The body under the hover preview: the title, one meta line and the play
// action. The board opens the window from here too (the module handles the
// double click); a single click on a control keeps its own meaning.
function previewBody(item) {
  const title = itemTitle(item);
  const meta = previewMeta({
    kindLabel: KIND_LABELS[item.media.kind] || item.media.kind,
    duration: formatDuration(shownDuration(item)) || '',
    author: item.createdByName || '',
  });
  return [
    h('p', { class: 'hp-title', title }, title),
    h('p', { class: 'hp-facts' }, meta),
    can('play')
      ? h(
          'button',
          {
            type: 'button',
            class: 'btn is-air is-wide hp-send',
            'aria-label': `Jouer « ${title} » sur les overlays`,
            title: 'Jouer le média sur les overlays',
            onClick: (event) => {
              event.stopPropagation();
              playItem(item, event.currentTarget);
            },
          },
          h('span', { class: 'hp-send-glyph' }, icon('play')),
          h('span', { class: 'hp-send-label' }, 'Jouer sur les overlays'),
        )
      : null,
  ];
}

function addedDateLabel(item) {
  const date = new Date(item.createdAt);
  return Number.isNaN(date.getTime())
    ? null
    : date.toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: 'numeric' });
}

function authorName(item) {
  return item.createdByName || 'Inconnu';
}

// One action of the information column. Which ones exist, and their order, is
// detailActionIds()'s decision; each node only wires the real handler.
function detailActionNode(item, action) {
  switch (action) {
    case 'play':
      return h(
        'button',
        { type: 'button', class: 'btn is-air is-wide hp-page-send', onClick: (event) => playItem(item, event.currentTarget) },
        icon('play'),
        'Jouer sur les overlays',
      );
    case 'edit':
      return h(
        'button',
        { type: 'button', class: 'btn is-quiet', title: 'Découper, recadrer, dessiner…', onClick: () => openMediaEditor(item, itemTitle(item)) },
        icon('edit'),
        'Modifier',
      );
    case 'refresh':
      return h(
        'button',
        { type: 'button', class: 'btn is-quiet', title: 'Rafraîchir le média depuis sa source', onClick: () => refreshSource(item) },
        icon('refresh'),
        'Rafraîchir depuis la source',
      );
    case 'import':
      return h(
        'button',
        { type: 'button', class: 'btn is-quiet', onClick: () => openImportDialog(menuItemIds(item), leaveSelection) },
        'Ajouter à une autre guilde…',
      );
    case 'delete':
      return h(
        'button',
        { type: 'button', class: 'btn is-quiet is-danger', onClick: () => openDeleteDialog(item) },
        icon('remove'),
        'Supprimer…',
      );
    default:
      return null;
  }
}

// The « Durée d’affichage » field being edited survives a board refresh, like
// the inline title and caption: a refresh rebuilds the column through
// detailBody and removes the field from the page.
let displayDurationDraft = null;

// The information column of the detail window: the media's own facts, its
// inline title and caption, its shortcut, the display time and picture of what
// the user can edit, and the actions the context menu offered. The facts and
// the actions are pinned in the column's footer, so a long column never hides
// them. Every change applies at once, like the inline fields. A board refresh
// rebuilds only this node.
function detailBody(item) {
  if (inlineEdit && inlineEdit.itemId !== item.id) {
    inlineEdit = null;
  }
  if (displayDurationDraft && displayDurationDraft.itemId !== item.id) {
    displayDurationDraft = null;
  }
  const kindLabel = KIND_LABELS[item.media.kind] || item.media.kind;
  const duration = formatDuration(shownDuration(item));
  const link = sourceLink(item);
  const canPlay = can('play');
  const isImage = item.media.kind === 'image';
  const isAudio = item.media.kind === 'audio';
  const size = item.media.sizeBytes ? `${(item.media.sizeBytes / 1048576).toFixed(1).replace('.', ',')} Mo` : null;
  const width = Number(item.media.width);
  const height = Number(item.media.height);
  const dimensions = width > 0 && height > 0 ? `${width} × ${height}` : '—';
  const scale = item.displayScalePercent ? `${item.displayScalePercent} %` : null;
  const actions = detailActionIds({
    canPlay,
    canOpenEditor: canOpenEditor(item),
    canRefresh: Boolean(link && can('addMedia') && item.media.edited !== true),
    canImport: importTargets(state.guilds, state.activeGuildId).length > 0,
    canDelete: Boolean(item.canEdit),
  });
  const added = addedDateLabel(item);
  const author = authorName(item);
  const kicker = [kindLabel, duration].filter(Boolean).join(' · ');
  const addedBy = `Ajouté par ${author}${added ? ` le ${added}` : ''}`;

  // Display time of an image: a fact row edited in place. Enter or leaving the
  // field saves it, Escape puts the stored value back and keeps the window
  // open, an invalid value keeps the field focused with its text. The draft
  // outlives a rebuild so a refresh while typing loses neither the text nor
  // the focus.
  function displayDurationField() {
    const draft = displayDurationDraft && displayDurationDraft.itemId === item.id ? displayDurationDraft : null;
    const input = h('input', {
      class: 'field display-duration',
      type: 'number',
      min: '1',
      max: String(MAX_DISPLAY_DURATION_SEC),
      step: '1',
      inputmode: 'numeric',
      value: draft ? draft.value : item.displayDurationSec ?? '',
      placeholder: defaultDurationHint(),
      'aria-label': 'Durée d’affichage (s)',
    });
    let settled = false;
    // Focus and typing record the draft: a refresh rebuilds this field and puts
    // the value and the caret back where the user left them.
    const remember = () => {
      displayDurationDraft = {
        itemId: item.id,
        value: input.value,
        selectionStart: input.selectionStart,
        selectionEnd: input.selectionEnd,
      };
    };
    const forget = () => {
      if (displayDurationDraft && displayDurationDraft.itemId === item.id) {
        displayDurationDraft = null;
      }
    };
    const save = async () => {
      if (settled) {
        return;
      }
      const parsed = parseDisplayDuration(input.value);
      if (!parsed.ok) {
        notify(INVALID_DURATION_MESSAGE, true);
        input.focus();
        return;
      }
      if (parsed.value === (item.displayDurationSec ?? null)) {
        settled = true;
        forget();
        return;
      }
      settled = true;
      input.disabled = true;
      const body = await run(() =>
        api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, { displayDurationSec: parsed.value }),
      );
      if (!body) {
        input.disabled = false;
        settled = false;
        return;
      }
      item.displayDurationSec = body.item && 'displayDurationSec' in body.item ? body.item.displayDurationSec : parsed.value;
      forget();
      notify('Média modifié.');
      loadItems({ quiet: true });
    };
    input.addEventListener('input', remember);
    input.addEventListener('focus', remember);
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') {
        // Back to the stored value; the window stays open.
        event.preventDefault();
        forget();
        input.value = item.displayDurationSec ?? '';
        settled = false;
      } else if (event.key === 'Enter' && !event.isComposing) {
        event.preventDefault();
        save();
      }
    });
    // A re-render removes the field without the user leaving it: only a real
    // blur (the field still in the page) saves, like inlineInput.
    input.addEventListener('blur', () => {
      setTimeout(() => {
        if (input.isConnected) {
          forget();
          save();
        }
      });
    });
    if (draft) {
      queueMicrotask(() => {
        input.focus({ preventScroll: true });
        // A number field exposes no text selection: put the caret back only
        // when the browser offers one.
        if (draft.selectionStart !== null && draft.selectionEnd !== null) {
          try {
            input.setSelectionRange(draft.selectionStart, draft.selectionEnd);
          } catch {
            // The field's type refused the selection: focus alone is enough.
          }
        }
      });
    }
    return input;
  }

  // Picture of an audio: preview or audio mark, the chosen picture is uploaded
  // at once, « Retirer » deletes it at once and the switch PATCHes its flag.
  // A drop or a paste on the block belongs to the picture and stops there.
  function artworkSection() {
    const current = artworkOf(item);
    let busy = false;
    const preview = h('img', {
      class: 'artwork-preview',
      alt: '',
      draggable: 'false',
      hidden: !current,
      src: current ? current.url : null,
    });
    // Without a picture the zone shows an audio mark, never an empty image.
    const empty = h('span', { class: 'artwork-preview is-empty', 'aria-hidden': 'true', hidden: Boolean(current) }, icon('audio'));
    const name = h('span', { class: 'meta' }, current ? 'Image actuelle' : 'Aucune image');
    const fileInput = h('input', {
      type: 'file',
      accept: 'image/*',
      hidden: true,
      onChange: () => {
        chooseFrom(fileInput.files);
        fileInput.value = '';
      },
    });
    const chooseButton = h('button', { class: 'btn', type: 'button', onClick: () => fileInput.click() }, 'Choisir une image');
    const removeButton = h('button', { class: 'btn is-quiet', type: 'button', hidden: !current, onClick: () => removePicture() }, 'Retirer');
    const linkField = h('input', {
      class: 'field',
      type: 'url',
      placeholder: 'https://…',
      'aria-label': 'Lien d’une image',
      onKeydown: (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          importLink(linkField.value);
        }
      },
    });
    const importButton = h('button', { class: 'btn', type: 'button', onClick: () => importLink(linkField.value) }, 'Importer');
    const onOverlay = h('input', {
      type: 'checkbox',
      checked: Boolean(item.artworkOnOverlay),
      onChange: () => saveOverlay(),
    });
    const zone = h(
      'div',
      { class: 'artwork-drop' },
      preview,
      empty,
      h(
        'div',
        { class: 'artwork-drop-body' },
        name,
        h('div', { class: 'row' }, chooseButton, removeButton),
      ),
      fileInput,
    );
    const controls = [chooseButton, removeButton, importButton, linkField, onOverlay];
    const setBusy = (value) => {
      busy = value;
      for (const control of controls) {
        control.disabled = value;
      }
      zone.classList.toggle('is-busy', value);
    };
    const changed = () => {
      notify('Média modifié.');
      loadItems({ quiet: true });
    };
    const upload = async (file) => {
      if (busy) {
        return;
      }
      setBusy(true);
      const uploaded = await uploadArtwork(item, file);
      setBusy(false);
      if (uploaded) {
        changed();
      }
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
        upload(picked.file);
      }
      return true;
    };
    // A link is downloaded here, in the WebView, then sent like a file; the
    // shell downloads it when the site refuses a cross-origin fetch.
    const importLink = async (link) => {
      if (busy) {
        return;
      }
      setBusy(true);
      const label = name.textContent;
      name.textContent = 'Téléchargement de l’image…';
      const result = await fetchArtwork(link, globalThis.fetch, downloadArtworkLink);
      if (result.error) {
        name.textContent = label;
        setBusy(false);
        notify(result.error, true);
        return;
      }
      const uploaded = await uploadArtwork(item, result.file);
      setBusy(false);
      if (!uploaded) {
        name.textContent = label;
        return;
      }
      linkField.value = '';
      changed();
    };
    const removePicture = async () => {
      if (busy || !current) {
        return;
      }
      setBusy(true);
      const removed = await run(() => api('DELETE', `/overlay/media-board/items/${encodeURIComponent(item.id)}/artwork`));
      setBusy(false);
      if (!removed) {
        return;
      }
      item.artwork = null;
      changed();
    };
    const saveOverlay = async () => {
      if (busy) {
        return;
      }
      const checked = onOverlay.checked;
      if (checked === Boolean(item.artworkOnOverlay)) {
        return;
      }
      setBusy(true);
      const body = await run(() =>
        api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, { artworkOnOverlay: checked }),
      );
      setBusy(false);
      if (!body) {
        onOverlay.checked = Boolean(item.artworkOnOverlay);
        return;
      }
      item.artworkOnOverlay = checked;
      changed();
    };
    const carriesPicture = (event) => {
      const types = Array.from((event.dataTransfer && event.dataTransfer.types) || []);
      return types.includes('Files') || types.includes('text/uri-list') || types.includes('text/html') || types.includes('text/plain');
    };
    const onDrag = (event) => {
      if (!carriesPicture(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      if (!busy) {
        event.dataTransfer.dropEffect = 'copy';
        zone.classList.add('is-over');
      }
    };
    const section = h(
      'div',
      {
        class: 'hp-page-artwork',
        onDragenter: onDrag,
        onDragover: onDrag,
        onDragleave: (event) => {
          if (!(event.relatedTarget instanceof Node) || !section.contains(event.relatedTarget)) {
            zone.classList.remove('is-over');
          }
        },
        onDrop: (event) => {
          if (!carriesPicture(event)) {
            return;
          }
          // The picture owns this drop: it never reaches the board's own
          // handlers, which would import a new media.
          event.preventDefault();
          event.stopPropagation();
          zone.classList.remove('is-over');
          if (busy) {
            return;
          }
          const transfer = event.dataTransfer;
          if (chooseFrom(transfer.files)) {
            return;
          }
          const dropped = droppedArtworkLink({
            html: transfer.getData('text/html'),
            uriList: transfer.getData('text/uri-list'),
            text: transfer.getData('text/plain'),
          });
          if (dropped) {
            importLink(dropped);
          } else {
            notify(ARTWORK_MESSAGES.nothingDropped, true);
          }
        },
        onPaste: (event) => {
          const data = event.clipboardData;
          if (!data) {
            return;
          }
          if (busy) {
            event.preventDefault();
            return;
          }
          // A copied image is used as it is; a copied link is imported.
          if (chooseFrom(data.files)) {
            event.preventDefault();
            return;
          }
          const pasted = droppedArtworkLink({ text: data.getData('text/plain') });
          if (pasted) {
            event.preventDefault();
            linkField.value = pasted;
            importLink(pasted);
          }
        },
      },
      h('p', { class: 'hp-page-label' }, 'Image'),
      zone,
      h('p', { class: 'artwork-hint' }, 'Dépose une image ou un lien ici, ou colle le lien d’une image :'),
      h('div', { class: 'artwork-link' }, linkField, importButton),
      h('label', { class: 'switch' }, onOverlay, 'Afficher l’image sur les overlays pendant la lecture'),
    );
    return section;
  }

  // The facts, in the validated order. A missing value keeps its row so the
  // two columns stay aligned; a scale only exists when one was chosen.
  const facts = [];
  const fact = (label, value) => {
    facts.push(h('dt', null, label), h('dd', null, value));
  };
  fact('Format', [kindLabel, duration].filter(Boolean).join(' · '));
  fact('Taille', size || '—');
  fact('Dimensions', dimensions);
  if (scale) {
    fact('Échelle', scale);
  }
  if (link) {
    facts.push(
      h('dt', null, 'Source'),
      h(
        'dd',
        { class: 'source-cell' },
        h('span', { class: 'source-url', title: link }, link),
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
    );
  }
  if (canPlay) {
    facts.push(
      h('dt', null, 'Raccourci'),
      h(
        'dd',
        { class: 'shortcut-cell' },
        shortcutControl(item, shortcutFor(item.id)),
      ),
    );
  }
  if (isImage && item.canEdit) {
    fact('Durée d’affichage', displayDurationField());
  }

  // The caption sent with the next trigger, in its own block above the pinned
  // facts. It is a local draft, not an item field: everyone allowed to trigger
  // can type one, and an empty draft keeps the field reachable without a
  // visible empty frame.
  const legendText = inlineField(item, 'message', canPlay);

  // The play button keeps its own block, full width; the other actions are
  // quiet buttons in a two-column grid under a thin separator.
  const secondaryActions = actions.filter((action) => action !== 'play');

  return h(
    'div',
    null,
    h(
      'div',
      { class: 'hp-page-head' },
      h(
        'p',
        { class: 'hp-page-kicker', title: `${kicker} · ${addedBy}` },
        kicker,
        ' · Ajouté par ',
        h('strong', null, author),
        added ? ` le ${added}` : null,
      ),
      inlineField(item, 'title'),
    ),
    h(
      'section',
      { class: 'hp-page-legend-block' },
      h('p', { class: 'hp-page-label' }, 'Légende'),
      legendText,
    ),
    isAudio && item.canEdit ? artworkSection() : null,
    // The facts and the actions are pinned together at the bottom of the
    // column: the rest of the column scrolls under them, so an action is never
    // hidden by a page that is too tall and the facts always stay just above
    // the buttons.
    h(
      'div',
      { class: 'hp-page-footer' },
      h('dl', { class: 'hp-page-facts' }, facts),
      actions.length
        ? h(
            'div',
            { class: 'hp-page-actionzone' },
            actions.includes('play')
              ? h('div', { class: 'hp-page-actions' }, detailActionNode(item, 'play'))
              : null,
            secondaryActions.length
              ? h(
                  'div',
                  { class: 'hp-page-secondary' },
                  ...secondaryActions.map((action) => detailActionNode(item, action)),
                )
              : null,
          )
        : null,
    ),
  );
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
  // The caption is local by design: committing it only moves the text into the
  // draft map, never to the item (nor to the server). It is sent with the next
  // trigger and dropped by `playItem` or by the window's close.
  if (field === 'message') {
    const caption = value.trim();
    legendDrafts.delete(item.id);
    if (caption) {
      legendDrafts.set(item.id, caption);
    }
    syncDetail();
    return;
  }
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
    class: `field inline-input is-${field} ${isTitle ? 'hp-page-title' : 'hp-page-legend'}`,
    maxlength: spec.max,
    rows: isTitle ? null : '3',
    'aria-label': `Modifier ${spec.label}`,
    value: inlineEdit.value,
  });
  const captionBefore = isTitle ? '' : legendDraft(item.id);
  let settled = false;
  let captionBlur = false;
  const finishCaptionBlur = () => {
    captionBlur = false;
    if (!input.isConnected || document.activeElement === input) {
      return;
    }
    if (inlineEdit && inlineEdit.itemId === item.id && inlineEdit.field === 'message') {
      inlineEdit = null;
    }
    if (mediaDetail.openId() === item.id) {
      syncDetail();
    }
  };
  const settle = (save) => {
    if (settled) {
      return;
    }
    settled = true;
    if (save) {
      commitInline(item, field, input.value);
    } else {
      // Escape cancels the edit: restore the caption the window opened with,
      // including the typed text that the input listener moved into the draft.
      if (!isTitle) {
        if (captionBefore) {
          legendDrafts.set(item.id, captionBefore);
        } else {
          legendDrafts.delete(item.id);
        }
      }
      inlineEdit = null;
      syncDetail();
    }
  };
  input.addEventListener('input', () => {
    inlineEdit.value = input.value;
    if (!isTitle) {
      // Keep the draft current while typing: a click on « Lire » before the
      // field's deferred blur commit still sends the text on screen.
      legendDrafts.set(item.id, input.value);
    }
  });
  input.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      // Handled here: the window stays open.
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
      if (!input.isConnected || settled) {
        return;
      }
      if (!isTitle) {
        // Commit the caption now, but rebuild the column only after the click
        // that took the focus: rebuilding from `blur` used to replace « Lire »
        // between mousedown and mouseup, so the first click was lost.
        const caption = input.value.trim();
        legendDrafts.delete(item.id);
        if (caption) {
          legendDrafts.set(item.id, caption);
        }
        if (!captionBlur) {
          captionBlur = true;
          afterNextClick(document, finishCaptionBlur, {
            ignore: (event) =>
              input.contains(event.target) ||
              Boolean(inlineEdit && inlineEdit.itemId === item.id && inlineEdit.field !== 'message'),
          });
        }
        return;
      }
      settle(true);
    });
  });
  queueMicrotask(() => {
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  });
  return input;
}

function inlineField(item, field, available = true) {
  const spec = INLINE_FIELDS[field];
  const isTitle = field === 'title';
  const text = isTitle ? itemTitle(item) : legendDraft(item.id);
  // The window's text classes: the title keeps the page heading, the caption
  // the legend line. The inline-edit control adds its own classes on top.
  const textClass = isTitle ? 'hp-page-title' : 'hp-page-legend';
  if (isTitle ? !item.canEdit : !available) {
    if (isTitle) {
      return h('h2', { class: textClass }, text);
    }
    return text ? h('p', { class: textClass }, text) : null;
  }
  if (inlineEdit && inlineEdit.itemId === item.id && inlineEdit.field === field) {
    return inlineInput(item, field);
  }
  return h(
    'button',
    {
      class: `inline-edit is-${field} ${textClass}${text ? '' : ' is-empty'}`,
      type: 'button',
      title: `Modifier ${spec.label}`,
      onClick: () => {
        inlineEdit = {
          itemId: item.id,
          field,
          value: isTitle ? item[field] || '' : legendDraft(item.id),
        };
        syncDetail();
      },
    },
    h('span', { class: `inline-text${isTitle ? ' is-title' : ''}` }, text || spec.empty),
    icon('edit'),
  );
}

// The window's player follows the device's default level, like the board's
// other media players did.
document.addEventListener('livechat:sound', () => {
  mediaDetail.setVolume(mediaLevel(state.activeGuildId));
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
// The shell downloads a picture link the WebView cannot read: a site without a
// CORS header refuses a cross-origin fetch, while the shell has no origin.
function downloadArtworkLink(url) {
  return invoke('media_artwork_link', { payload: { url } });
}

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

export function openDeleteDialog(item) {
  confirmDialog('Supprimer ce média ?', `« ${itemTitle(item)} » sera retiré du board de la guilde.`, 'Supprimer', async () => {
    await run(() => api('DELETE', `/overlay/media-board/items/${encodeURIComponent(item.id)}`), 'Média supprimé.');
    closeDetail();
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
// A drop lands on the open « Ajouter un média » dialog when there is one (it
// covers the board, so its form is what the user is looking at), and on the
// board's main area otherwise. The veil names the destination.
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

// Where this drop lands, if anything takes it. The dialog comes first: its
// backdrop is outside the main area, so a drop on it used to be refused and
// nothing happened.
function dropDestination(target) {
  return dropTarget({ dialogOpen: addDialogOpen(), onBoard: canDropFiles() && inMainArea(target) });
}

function ensureDropVeil() {
  if (!dropVeil) {
    dropVeil = h(
      'div',
      { class: 'drop-veil', hidden: true, 'aria-hidden': 'true' },
      h('div', { class: 'drop-veil-inner' }, icon('plus'), h('p', null, DROP_VEIL_TEXT.board)),
    );
    document.body.append(dropVeil);
  }
  return dropVeil;
}

// The veil is the same element for both destinations: over the dialog it takes
// `is-modal` (the whole window, above the dialog's backdrop) and the sentence
// of what it fills. `dropLanding` remembers it for the dragleave below.
let dropLanding = null;

function showDropVeil(destination) {
  if (dropDismissed) {
    return;
  }
  const veil = ensureDropVeil();
  veil.classList.toggle('is-modal', destination === 'dialog');
  veil.querySelector('p').textContent = DROP_VEIL_TEXT[destination];
  // The fade behind the dialog takes the veil's dim; the veil itself is only
  // its frame and sentence over it.
  highlightAddDialog(destination === 'dialog');
  dropLanding = destination;
  veil.hidden = false;
}

function hideDropVeil() {
  highlightAddDialog(false);
  if (dropVeil) {
    dropVeil.hidden = true;
  }
}

function resetDropVeil() {
  dropDismissed = false;
  dropLanding = null;
  hideDropVeil();
}

// Never let the WebView open a dropped file, anywhere in the window.
document.addEventListener('dragover', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  const destination = dropDestination(event.target);
  if (destination) {
    event.dataTransfer.dropEffect = 'copy';
    showDropVeil(destination);
  }
});

document.addEventListener('dragenter', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  const destination = dropDestination(event.target);
  if (destination) {
    showDropVeil(destination);
  }
});

// Only a real exit hides the veil: moving between the children of the surface
// the veil covers keeps a relatedTarget inside it. Over the dialog the surface
// is the whole window, so only a drag that leaves the window exits it.
document.addEventListener('dragleave', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  const related = event.relatedTarget;
  const inside = dropLanding === 'dialog'
    ? related instanceof Node
    : related instanceof Node && inMainArea(related);
  if (!inside) {
    resetDropVeil();
  }
});

document.addEventListener('drop', (event) => {
  if (!transfersFiles(event)) {
    return;
  }
  event.preventDefault();
  const destination = dropDestination(event.target);
  resetDropVeil();
  if (!destination) {
    return;
  }
  // The dialog fills itself from the transfer: files are added to the ones
  // already chosen there, a link fills the link field.
  if (destination === 'dialog') {
    dropOnAddDialog(event.dataTransfer);
    return;
  }
  const files = droppedFiles(event);
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
