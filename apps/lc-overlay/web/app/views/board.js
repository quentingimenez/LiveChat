// Media board: grid, cards, detail panel, play, shortcuts, edit and delete.

import { api, mediaUrl, run } from '../core/api.js';
import { h, icon } from '../core/dom.js';
import { formatDuration } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { activeGuild, can, describeError, KIND_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, openDialog } from '../core/ui.js';
import { loadItems, refreshQueueAutomatically } from '../data.js';
import { openAddDialog } from './add-media.js';
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
      const { assetId, kind } = entry.target.dataset;
      mediaUrl(assetId)
        .then((url) => {
          const media =
            kind === 'video'
              ? h('video', { src: `${url}#t=0.1`, preload: 'metadata', muted: true, playsinline: true, tabindex: '-1' })
              : h('img', { src: url, alt: '', loading: 'lazy' });
          // The fallback glyph goes away as soon as the thumbnail can be shown.
          const glyph = entry.target.querySelector('.glyph');
          media.addEventListener('error', () => media.remove());
          media.addEventListener(kind === 'video' ? 'loadeddata' : 'load', () => glyph && glyph.remove());
          entry.target.prepend(media);
        })
        .catch(() => {});
    }
  },
  { rootMargin: '200px' },
);

function renderCard(item) {
  const kind = item.media.kind;
  const thumb = h(
    'button',
    {
      class: 'thumb',
      'data-asset-id': item.mediaAssetId,
      'data-kind': kind,
      'aria-label': `Détails de ${itemTitle(item)}`,
      onClick: () => selectItem(item.id),
    },
    h('span', { class: 'glyph' }, icon(kind === 'audio' ? 'audio' : kind === 'image' ? 'image' : 'video')),
    formatDuration(item.media.durationSec) ? h('span', { class: 'duration' }, formatDuration(item.media.durationSec)) : null,
  );
  if (kind !== 'audio') {
    thumbObserver.observe(thumb);
  }
  return h(
    'div',
    { class: `card${state.selectedId === item.id ? ' is-selected' : ''}` },
    h(
      'div',
      { style: 'position:relative' },
      thumb,
      can('play')
        ? h('button', { class: 'play', 'aria-label': `Jouer ${itemTitle(item)}`, title: 'Jouer sur les overlays', onClick: () => playItem(item) }, icon('play'))
        : null,
    ),
    h('p', { class: 'card-title', title: itemTitle(item) }, itemTitle(item)),
  );
}

export function renderBoard() {
  if (state.info && !can('viewBoard')) {
    return h('div', { class: 'empty' }, h('h2', null, 'Board réservé aux membres'), h('p', null, "Cette guilde ne montre pas son board aux invités. Tu reçois quand même les médias diffusés."));
  }
  const items = visibleItems();
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
  if (!state.itemsLoaded) {
    content = h('p', { class: 'meta' }, 'Chargement du board…');
  } else if (state.items.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Le board est vide'),
      h('p', null, 'Colle le lien d’une vidéo, d’une image ou d’un son pour créer le premier média.'),
      can('addMedia') ? h('button', { class: 'btn', onClick: openAddDialog }, 'Ajouter un média') : null,
    );
  } else if (items.length === 0) {
    content = h('p', { class: 'meta' }, 'Aucun média ne correspond à cette recherche.');
  } else {
    content = h('div', { class: 'grid', id: 'grid' }, items.map(renderCard));
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
  if (!grid || items.length === 0) {
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
  grid.replaceChildren(...items.map(renderCard));
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
  const response = await run(() => invoke('media_board_trigger', { payload: { itemId: item.id, guildId: state.activeGuildId, trigger: 'ui' } }));
  if (!response) {
    return;
  }
  if (response.ok) {
    notify(`« ${itemTitle(item)} » envoyé aux overlays.`);
    setTimeout(refreshQueueAutomatically, 400);
  } else {
    notify(describeError(response.reason), true);
  }
}

export function shortcutFor(itemId) {
  const entry = Object.entries(state.bindings).find(([, id]) => id === itemId);
  return entry ? entry[0] : null;
}

function renderDetail(item) {
  const preview = h('div', { class: 'preview' });
  mediaUrl(item.mediaAssetId)
    .then((url) => {
      if (item.media.kind === 'video') {
        preview.append(h('video', { src: url, controls: true, preload: 'metadata' }));
      } else if (item.media.kind === 'audio') {
        preview.append(h('audio', { src: url, controls: true }));
      } else {
        preview.append(h('img', { src: url, alt: '' }));
      }
    })
    .catch(() => {});
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
                  ? h('kbd', null, shortcut.replace('CommandOrControl', 'Ctrl'))
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

function acceleratorFromEvent(event) {
  const key = event.key;
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(key)) {
    return null;
  }
  const parts = [];
  if (event.ctrlKey || event.metaKey) {
    parts.push('CommandOrControl');
  }
  if (event.altKey) {
    parts.push('Alt');
  }
  if (event.shiftKey) {
    parts.push('Shift');
  }
  if (parts.length === 0) {
    return undefined; // a bare key would capture typing in every application
  }
  let name = null;
  if (/^Key[A-Z]$/.test(event.code)) {
    name = event.code.slice(3);
  } else if (/^Digit[0-9]$/.test(event.code)) {
    name = event.code.slice(5);
  } else if (/^Numpad[0-9]$/.test(event.code)) {
    name = `num${event.code.slice(6)}`;
  } else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) {
    name = key;
  }
  return name ? [...parts, name].join('+') : undefined;
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
