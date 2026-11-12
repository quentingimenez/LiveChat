// LiveChat — application principale de l'overlay (fenêtre « board »).
//
// Module ES sans build ni dépendance (CSP `script-src 'self'`). Le jeton d'une
// guilde ne quitte jamais le shell Rust : toutes les requêtes REST passent par
// la commande `guild_api`. Aucune donnée distante n'est injectée en HTML : le
// DOM est construit nœud par nœud (`h`).

// ------------------------------------------------------------------- IPC ---
const internals = window.__TAURI_INTERNALS__ || null;

function invoke(command, args) {
  if (!internals) {
    return Promise.reject(new Error('Application ouverte hors de LiveChat.'));
  }
  return internals.invoke(command, args);
}

function listen(eventName, handler) {
  if (!internals) {
    return;
  }
  const id = internals.transformCallback((event) => handler(event && event.payload), false);
  invoke('plugin:event|listen', { event: eventName, target: { kind: 'Any' }, handler: id }).catch(() => {});
}

// ------------------------------------------------------------------- DOM ---
function h(tag, props, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props || {})) {
    if (value === null || value === undefined || value === false) {
      continue;
    }
    if (key === 'class') {
      node.className = value;
    } else if (key.startsWith('on')) {
      node.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === 'value' || key === 'checked' || key === 'disabled' || key === 'selected' || key === 'muted') {
      node[key] = value;
    } else {
      node.setAttribute(key, value === true ? '' : String(value));
    }
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) {
      continue;
    }
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

const ICONS = {
  play: 'M7 4.5v15l12-7.5z',
  stop: 'M6 6h12v12H6z',
  plus: 'M12 5v14M5 12h14',
  close: 'M6 6l12 12M18 6L6 18',
  video: 'M4 6h11v12H4zM15 10l5-3v10l-5-3',
  image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4',
  audio: 'M9 18V6l10-2v12M9 18a3 3 0 11-6 0 3 3 0 016 0zM19 16a3 3 0 11-6 0 3 3 0 016 0z',
  copy: 'M9 9h10v10H9zM5 15V5h10',
  mute: 'M4 9v6h4l5 4V5L8 9zM17 9l4 6M21 9l-4 6',
  sound: 'M4 9v6h4l5 4V5L8 9zM16 9a4 4 0 010 6',
  remove: 'M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13',
  refresh: 'M5 12a7 7 0 0112-5l2 2M19 5v4h-4M19 12a7 7 0 01-12 5l-2-2M5 19v-4h4',
};

function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', name === 'play' || name === 'stop' ? 'currentColor' : 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', '1.8');
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', 'icon');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', ICONS[name] || '');
  svg.append(path);
  return svg;
}

// ------------------------------------------------------------------ état ---
const state = {
  guilds: [],
  activeGuildId: null,
  profile: { displayName: null, avatar: null },
  tab: 'board',
  adminTab: 'members',
  info: null, // réponse de GET /overlay/guild pour la guilde active
  items: [],
  itemsLoaded: false,
  search: '',
  kind: 'all',
  selectedId: null,
  queue: { playing: null, pending: [] },
  members: [],
  invites: [],
  bindings: {},
  mediaUrls: new Map(),
  capturing: false,
};

const ROLE_LABELS = { admin: 'Admin', member: 'Membre', guest: 'Invité' };
const KIND_LABELS = { all: 'Tous', video: 'Vidéo', image: 'Image', audio: 'Audio' };
const ERROR_LABELS = {
  forbidden: "Ton rôle ne permet pas cette action.",
  read_only_session: 'Les invités ne peuvent pas modifier le board.',
  last_admin: "Tu es le dernier admin : nomme un autre admin d'abord.",
  unauthorized: "Cet appareil n'est plus reconnu par la guilde.",
  pairing_code_invalid_or_expired: 'Code invalide, expiré ou déjà utilisé.',
  pairing_code_scope_mismatch: "Ce code sert à appairer l'extension navigateur, pas un overlay.",
  guild_not_standalone: 'Seule une guilde créée hors Discord peut être liée à un serveur Discord.',
  member_required: 'Action réservée aux membres de la guilde.',
  invalid_role: 'Rôle inconnu.',
  invalid_payload: 'Valeur invalide.',
  invalid_name: 'Nom invalide.',
  board_capacity_exceeded: 'Le board de la guilde est plein.',
  socket_unavailable: 'Guilde déconnectée : impossible de jouer pour le moment.',
};

function can(permission) {
  return Boolean(state.info && state.info.permissions && state.info.permissions[permission]);
}

function activeGuild() {
  return state.guilds.find((guild) => guild.membership.guildId === state.activeGuildId) || null;
}

function guildLabel(guild) {
  return guild.membership.guildName || (state.info && state.info.guild && state.info.guild.name) || 'Guilde';
}

// ----------------------------------------------------------------- notices ---
let noticeTimer = null;

function notify(message, isError) {
  const node = document.getElementById('notice');
  node.textContent = message;
  node.classList.toggle('is-error', Boolean(isError));
  node.hidden = false;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => {
    node.hidden = true;
  }, isError ? 7000 : 3500);
}

function describeError(code, fallback) {
  return ERROR_LABELS[code] || fallback || 'Action impossible.';
}

// --------------------------------------------------------------------- API ---
async function api(method, path, body) {
  const guildId = state.activeGuildId;
  const response = await invoke('guild_api', { payload: { guildId, method, path, body: body ?? null } });
  if (!response.ok) {
    const code = response.body && response.body.error;
    const error = new Error(describeError(code, `Le serveur a répondu ${response.status}.`));
    error.code = code;
    error.status = response.status;
    throw error;
  }
  return response.body || {};
}

async function run(action, successMessage) {
  try {
    const result = await action();
    if (successMessage) {
      notify(successMessage);
    }
    return result;
  } catch (error) {
    notify(typeof error === 'string' ? error : error.message || 'Action impossible.', true);
    return undefined;
  }
}

async function mediaUrl(assetId) {
  const key = `${state.activeGuildId}:${assetId}`;
  if (!state.mediaUrls.has(key)) {
    state.mediaUrls.set(key, await invoke('guild_media_url', { payload: { guildId: state.activeGuildId, assetId } }));
  }
  return state.mediaUrls.get(key);
}

// ------------------------------------------------------------- chargements ---
function applyGuilds(payload) {
  if (!payload) {
    return;
  }
  const previous = state.activeGuildId;
  state.guilds = payload.guilds || [];
  state.activeGuildId = payload.activeGuildId || null;
  state.profile = payload.profile || state.profile;
  if (previous !== state.activeGuildId) {
    resetGuildData();
    loadGuild();
  }
  renderRail();
  renderHeader();
  if (!state.activeGuildId) {
    renderView();
  }
}

function resetGuildData() {
  state.info = null;
  state.items = [];
  state.itemsLoaded = false;
  state.selectedId = null;
  state.queue = { playing: null, pending: [] };
  state.members = [];
  state.invites = [];
  state.search = '';
  state.kind = 'all';
  if (state.tab === 'admin') {
    state.tab = 'board';
  }
}

async function loadGuild() {
  if (!state.activeGuildId) {
    return;
  }
  const guildId = state.activeGuildId;
  try {
    const info = await api('GET', '/overlay/guild');
    if (guildId !== state.activeGuildId) {
      return;
    }
    state.info = info;
  } catch (error) {
    if (guildId === state.activeGuildId) {
      notify(error.message, true);
    }
  }
  const bindings = await invoke('meme_board_get_bindings').catch(() => null);
  state.bindings = (bindings && bindings.bindings) || {};
  renderHeader();
  await Promise.all([loadItems(), loadQueue()]);
}

async function loadItems() {
  if (!state.activeGuildId) {
    return;
  }
  const guildId = state.activeGuildId;
  try {
    const body = await api('GET', '/overlay/meme-board/items?limit=200');
    if (guildId !== state.activeGuildId) {
      return;
    }
    state.items = body.items || [];
  } catch (error) {
    state.items = [];
    if (error.code !== 'forbidden') {
      notify(error.message, true);
    }
  }
  state.itemsLoaded = true;
  if (state.tab === 'board') {
    renderView();
  }
}

let queueEndTimer = null;

function scheduleQueueEndRefresh() {
  clearTimeout(queueEndTimer);
  queueEndTimer = null;
  const playing = state.queue.playing;
  const started = playing && playing.startedAt ? new Date(playing.startedAt).getTime() : NaN;
  if (!Number.isFinite(started) || !(playing.durationSec > 0)) {
    return;
  }
  const endsAt = started + playing.durationSec * 1000;
  const delay = Math.max(0, endsAt - Date.now()) + 1500;
  queueEndTimer = setTimeout(loadQueue, delay);
}

async function loadQueue() {
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
  } catch {
    return;
  }
  renderHeader();
  renderOnAir();
  if (state.tab === 'queue') {
    renderView();
  }
  scheduleQueueEndRefresh();
}

async function loadAdmin() {
  if (state.adminTab === 'members' && can('manageMembers')) {
    const body = await run(() => api('GET', '/overlay/admin/members'));
    state.members = (body && body.items) || [];
  }
  if (state.adminTab === 'invites' && can('manageInvites')) {
    const body = await run(() => api('GET', '/overlay/admin/invites'));
    state.invites = (body && body.items) || [];
  }
  if (state.tab === 'admin') {
    renderView();
  }
}

// -------------------------------------------------------------------- rail ---
function initials(name) {
  const words = String(name || '?').trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || '?').slice(0, 2);
  return letters.toUpperCase();
}

function dotClass(guild) {
  if (guild.playing) {
    return 'is-air';
  }
  switch (guild.connection.status) {
    case 'connected':
      return 'is-on';
    case 'connecting':
    case 'reconnecting':
      return 'is-busy';
    case 'error':
    case 'revoked':
    case 'outdated':
      return 'is-err';
    default:
      return '';
  }
}

function renderRail() {
  const rail = document.getElementById('rail');
  rail.replaceChildren(
    ...state.guilds.map((guild) => {
      const label = guild.membership.guildName || 'Guilde';
      const active = guild.membership.guildId === state.activeGuildId;
      return h(
        'button',
        {
          class: `guild${active ? ' is-active' : ''}${guild.membership.muted ? ' is-muted' : ''}`,
          title: `${label} — ${guild.membership.muted ? 'en sourdine' : guild.statusLabel}`,
          'aria-current': active ? 'true' : null,
          onClick: () => selectGuild(guild.membership.guildId),
        },
        initials(label),
        h('span', { class: `dot ${dotClass(guild)}` }),
      );
    }),
    h('button', { class: 'guild is-add', title: 'Rejoindre une guilde', 'aria-label': 'Rejoindre une guilde', onClick: openJoinDialog }, '+'),
    h('span', { class: 'rail-spacer' }),
  );
}

async function selectGuild(guildId) {
  if (guildId === state.activeGuildId) {
    return;
  }
  const result = await run(() => invoke('guild_select', { payload: { guildId } }));
  if (result) {
    applyGuilds(result.guilds);
  }
}

// ------------------------------------------------------------------ header ---
function renderHeader() {
  const header = document.getElementById('header');
  const guild = activeGuild();
  if (!guild) {
    header.replaceChildren();
    header.hidden = true;
    return;
  }
  header.hidden = false;
  const role = (state.info && state.info.member && state.info.member.role) || guild.membership.role;
  const connected = state.info && state.info.guild ? state.info.guild.connectedCount : null;
  const tabs = [
    ['board', 'Board', null],
    ['queue', "File d'attente", state.queue.pending.length || null],
  ];
  if (can('manageMembers') || can('manageInvites') || can('manageSettings')) {
    tabs.push(['admin', 'Administration', null]);
  }
  header.replaceChildren(
    h(
      'div',
      { class: 'header-top' },
      h('h1', { class: 'guild-name' }, guildLabel(guild)),
      role ? h('span', { class: `badge is-${role}` }, ROLE_LABELS[role] || role) : null,
      guild.membership.muted ? h('span', { class: 'badge is-off' }, 'En sourdine') : null,
      guild.connection.status !== 'connected' && !guild.membership.muted
        ? h('span', { class: 'badge is-off' }, guild.statusLabel)
        : null,
      h('span', { class: 'spacer' }),
      connected !== null ? h('span', { class: 'meta' }, `${connected} overlay${connected > 1 ? 's' : ''} connecté${connected > 1 ? 's' : ''}`) : null,
      h(
        'button',
        {
          class: 'btn is-quiet',
          title: guild.membership.muted ? 'Recevoir à nouveau les médias de cette guilde' : 'Ne plus recevoir les médias de cette guilde',
          onClick: () => toggleMute(guild),
        },
        icon(guild.membership.muted ? 'mute' : 'sound'),
        guild.membership.muted ? 'Rétablir' : 'Sourdine',
      ),
      h('button', { class: 'btn is-quiet', onClick: () => openGuildDialog(guild) }, 'Mon profil'),
    ),
    h(
      'div',
      { class: 'tabs', role: 'tablist' },
      tabs.map(([id, label, count]) =>
        h(
          'button',
          { class: `tab${state.tab === id ? ' is-active' : ''}`, role: 'tab', 'aria-selected': state.tab === id, onClick: () => setTab(id) },
          label,
          count ? h('span', { class: 'count' }, count) : null,
        ),
      ),
    ),
  );
}

function setTab(tab) {
  state.tab = tab;
  renderHeader();
  renderView();
  if (tab === 'admin') {
    loadAdmin();
  }
  if (tab === 'queue') {
    loadQueue();
  }
}

async function toggleMute(guild) {
  const result = await run(() =>
    invoke('guild_set_muted', { payload: { guildId: guild.membership.guildId, muted: !guild.membership.muted } }),
  );
  if (result) {
    applyGuilds(result.guilds);
  }
}

// --------------------------------------------------------------------- vue ---
function renderView() {
  const view = document.getElementById('view');
  if (!state.activeGuildId) {
    view.replaceChildren(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, 'Rejoins ta première guilde'),
        h('p', null, "Un admin te donne un code. Il fixe ton rôle : admin, membre ou invité."),
        h('button', { class: 'btn is-air', onClick: openJoinDialog }, 'Saisir un code'),
      ),
    );
    return;
  }
  if (state.tab === 'queue') {
    view.replaceChildren(renderQueue());
  } else if (state.tab === 'admin') {
    view.replaceChildren(renderAdmin());
  } else {
    view.replaceChildren(renderBoard());
  }
}

// ------------------------------------------------------------------- board ---
function formatDuration(seconds) {
  if (!seconds || seconds <= 0) {
    return null;
  }
  const total = Math.round(seconds);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
}

function itemTitle(item) {
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

// Les vignettes ne chargent leur média qu'à l'approche de l'écran.
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
          // L'icône de repli disparaît dès que la vignette est affichable.
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

function renderBoard() {
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
      placeholder: 'Rechercher un mème',
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
    can('addMeme') ? h('button', { class: 'btn', onClick: openAddDialog }, icon('plus'), 'Ajouter un mème') : null,
  );
  let content;
  if (!state.itemsLoaded) {
    content = h('p', { class: 'meta' }, 'Chargement du board…');
  } else if (state.items.length === 0) {
    content = h(
      'div',
      { class: 'empty' },
      h('h2', null, 'Le board est vide'),
      h('p', null, 'Colle le lien d’une vidéo, d’une image ou d’un son pour créer le premier mème.'),
      can('addMeme') ? h('button', { class: 'btn', onClick: openAddDialog }, 'Ajouter un mème') : null,
    );
  } else if (items.length === 0) {
    content = h('p', { class: 'meta' }, 'Aucun mème ne correspond à cette recherche.');
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

function selectItem(id) {
  state.selectedId = state.selectedId === id ? null : id;
  state.capturing = false;
  renderView();
}

async function playItem(item) {
  const guild = activeGuild();
  if (guild && guild.membership.muted) {
    notify('Guilde en sourdine : rétablis-la pour jouer.', true);
    return;
  }
  const response = await run(() => invoke('meme_board_trigger', { payload: { itemId: item.id, guildId: state.activeGuildId, trigger: 'ui' } }));
  if (!response) {
    return;
  }
  if (response.ok) {
    notify(`« ${itemTitle(item)} » envoyé aux overlays.`);
    setTimeout(loadQueue, 400);
  } else {
    notify(describeError(response.reason), true);
  }
}

function shortcutFor(itemId) {
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
    { class: 'detail', 'aria-label': 'Détails du mème' },
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

// --- raccourcis clavier (guilde active) ---
let captureItem = null;

function startCapture(item) {
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
    return undefined; // une touche seule capturerait la frappe dans toutes les applications
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

async function saveShortcut(item, accelerator) {
  const bindings = {};
  for (const [existing, id] of Object.entries(state.bindings)) {
    if (id !== item.id && existing !== accelerator) {
      bindings[existing] = id;
    }
  }
  if (accelerator) {
    bindings[accelerator] = item.id;
  }
  const response = await run(() => invoke('meme_board_set_bindings', { payload: { bindings } }));
  if (response) {
    state.bindings = response.bindings || {};
    if (response.failedAccelerators && response.failedAccelerators.length > 0) {
      notify('Ce raccourci est déjà pris par une autre application.', true);
    }
  }
  renderView();
}

// --------------------------------------------------------------------- file ---
function jobTitle(job) {
  return job.title || job.text || (job.kind ? KIND_LABELS[job.kind] : null) || 'Média';
}

function renderQueue() {
  const { playing, pending } = state.queue;
  const blocks = [];
  if (playing) {
    const started = playing.startedAt ? new Date(playing.startedAt).getTime() : null;
    const elapsed = started ? Math.max(0, (Date.now() - started) / 1000) : 0;
    const ratio = playing.durationSec > 0 ? Math.min(1, elapsed / playing.durationSec) : 0;
    blocks.push(
      h(
        'div',
        { class: 'now' },
        h(
          'div',
          { class: 'row' },
          h('strong', { class: 'ellipsis' }, jobTitle(playing)),
          playing.authorName ? h('span', { class: 'meta' }, `par ${playing.authorName}`) : null,
          h('span', { class: 'spacer' }),
          h('span', { class: 'meta' }, `${formatDuration(Math.min(elapsed, playing.durationSec)) || '0:00'} / ${formatDuration(playing.durationSec) || '0:00'}`),
          can('stop') ? h('button', { class: 'btn', onClick: stopPlayback }, icon('stop'), 'Arrêter') : null,
        ),
        h('div', { class: 'progress' }, h('span', { style: `width:${Math.round(ratio * 100)}%` })),
      ),
    );
  }
  if (pending.length === 0) {
    blocks.push(
      h(
        'div',
        { class: 'empty' },
        h('h2', null, playing ? 'Rien d’autre en attente' : 'La file est vide'),
        h('p', null, 'Les médias envoyés à la guilde apparaissent ici avant de passer à l’écran.'),
      ),
    );
  } else {
    const total = pending.reduce((sum, job) => sum + (job.durationSec || 0), 0);
    blocks.push(
      h(
        'div',
        { class: 'list' },
        pending.map((job, index) =>
          h(
            'div',
            { class: 'line is-queue' },
            h('span', { class: 'meta' }, index + 1),
            h('div', { class: 'ellipsis' }, jobTitle(job), job.authorName ? h('span', { class: 'sub' }, `  par ${job.authorName}`) : null),
            h('span', { class: 'meta' }, formatDuration(job.durationSec) || ''),
            can('moderateQueue')
              ? h('button', { class: 'btn is-quiet', 'aria-label': `Retirer ${jobTitle(job)} de la file`, title: 'Retirer de la file', onClick: () => removeJob(job) }, icon('close'))
              : h('span'),
          ),
        ),
      ),
      h(
        'div',
        { class: 'row', style: 'margin-top:14px' },
        // Pending jobs often have no duration yet (it is fixed when playback starts).
        h('span', { class: 'meta' }, formatDuration(total) ? `${pending.length} en attente, ${formatDuration(total)} au total` : `${pending.length} en attente`),
        h('span', { class: 'spacer' }),
        can('moderateQueue') ? h('button', { class: 'btn is-danger', onClick: clearQueue }, 'Vider la file') : null,
      ),
    );
  }
  return h('div', null, blocks);
}

async function stopPlayback() {
  const response = await run(() => invoke('meme_board_stop', { payload: { guildId: state.activeGuildId } }));
  if (response && !response.ok) {
    notify(describeError(response.reason), true);
  }
  setTimeout(loadQueue, 400);
}

async function removeJob(job) {
  await run(() => api('DELETE', `/overlay/queue/${encodeURIComponent(job.id)}`));
  loadQueue();
}

async function clearQueue() {
  await run(() => api('DELETE', '/overlay/queue'), 'File vidée.');
  loadQueue();
}

function renderOnAir() {
  const bar = document.getElementById('onair');
  const playing = state.queue.playing;
  if (!playing || !state.activeGuildId) {
    bar.hidden = true;
    return;
  }
  bar.hidden = false;
  // replaceChildren() would render a null child as the text "null": filter first.
  const children = [
    h('span', { class: 'pulse' }),
    h('span', { class: 'title ellipsis' }, jobTitle(playing)),
    playing.authorName ? h('span', { class: 'meta' }, `par ${playing.authorName}`) : null,
    h('span', { class: 'spacer' }),
    state.queue.pending.length ? h('span', { class: 'meta' }, `${state.queue.pending.length} en attente`) : null,
    can('stop') ? h('button', { class: 'btn', onClick: stopPlayback }, icon('stop'), 'Arrêter') : null,
  ];
  bar.replaceChildren(...children.filter(Boolean));
}

// -------------------------------------------------------------------- admin ---
function renderAdmin() {
  const tabs = [];
  if (can('manageMembers')) {
    tabs.push(['members', 'Membres']);
  }
  if (can('manageInvites')) {
    tabs.push(['invites', 'Invitations']);
  }
  if (can('manageSettings')) {
    tabs.push(['settings', 'Réglages']);
  }
  if (!tabs.some(([id]) => id === state.adminTab)) {
    state.adminTab = tabs.length ? tabs[0][0] : 'members';
  }
  const body =
    state.adminTab === 'invites' ? renderInvites() : state.adminTab === 'settings' ? renderSettings() : renderMembers();
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'subtabs' },
      tabs.map(([id, label]) =>
        h('button', { class: `chip${state.adminTab === id ? ' is-active' : ''}`, onClick: () => { state.adminTab = id; renderView(); loadAdmin(); } }, label),
      ),
    ),
    body,
  );
}

function relativeTime(iso) {
  if (!iso) {
    return 'Jamais vu';
  }
  const minutes = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 2) {
    return "À l'instant";
  }
  if (minutes < 60) {
    return `Il y a ${minutes} min`;
  }
  if (minutes < 1440) {
    return `Il y a ${Math.round(minutes / 60)} h`;
  }
  return `Il y a ${Math.round(minutes / 1440)} j`;
}

function avatarNode(name, avatar) {
  return avatar ? h('img', { class: 'avatar', src: avatar, alt: '' }) : h('span', { class: 'avatar' }, initials(name).slice(0, 1));
}

function renderMembers() {
  if (state.members.length === 0) {
    return h('p', { class: 'meta' }, 'Chargement des membres…');
  }
  return h(
    'div',
    { class: 'list' },
    state.members.map((member) =>
      h(
        'div',
        { class: 'line is-member' },
        avatarNode(member.displayName, member.avatar),
        h('div', { class: 'ellipsis' }, member.displayName, member.isSelf ? h('span', { class: 'sub' }, '  (toi)') : null),
        h('span', { class: 'sub seen' }, member.connected ? 'En ligne' : relativeTime(member.lastSeenAt)),
        h(
          'select',
          { class: 'field', 'aria-label': `Rôle de ${member.displayName}`, onChange: (event) => changeRole(member, event.target.value) },
          Object.entries(ROLE_LABELS).map(([id, label]) => h('option', { value: id, selected: member.role === id }, label)),
        ),
        member.isSelf
          ? h('span')
          : h('button', { class: 'btn is-quiet is-danger', onClick: () => openKickDialog(member) }, 'Exclure'),
      ),
    ),
  );
}

async function changeRole(member, role) {
  await run(() => api('PATCH', `/overlay/admin/members/${encodeURIComponent(member.id)}`, { role }), `${member.displayName} est maintenant ${ROLE_LABELS[role].toLowerCase()}.`);
  await loadAdmin();
  if (member.isSelf) {
    loadGuild();
  }
}

function formatCode(code) {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}

function inviteStatus(invite) {
  if (invite.revokedAt) {
    return 'Révoquée';
  }
  const uses = invite.maxUses ? `${invite.useCount} / ${invite.maxUses} utilisations` : `${invite.useCount} utilisation${invite.useCount > 1 ? 's' : ''}`;
  if (!invite.isActive) {
    return invite.maxUses && invite.useCount >= invite.maxUses ? `Épuisée, ${uses}` : `Expirée, ${uses}`;
  }
  if (!invite.expiresAt) {
    return `Sans expiration, ${uses}`;
  }
  const hours = Math.max(1, Math.round((new Date(invite.expiresAt).getTime() - Date.now()) / 3600000));
  return `Expire dans ${hours >= 48 ? `${Math.round(hours / 24)} j` : `${hours} h`}, ${uses}`;
}

function renderInvites() {
  const role = h('select', { class: 'field', 'aria-label': 'Rôle accordé' }, [
    h('option', { value: 'member' }, 'Membre'),
    h('option', { value: 'guest' }, 'Invité'),
    h('option', { value: 'admin' }, 'Admin'),
  ]);
  const expiry = h('select', { class: 'field', 'aria-label': 'Expiration' }, [
    h('option', { value: '24' }, 'Expire dans 24 h'),
    h('option', { value: '168' }, 'Expire dans 7 jours'),
    h('option', { value: '' }, "N'expire jamais"),
  ]);
  const uses = h('select', { class: 'field', 'aria-label': "Nombre d'utilisations" }, [
    h('option', { value: '1' }, '1 utilisation'),
    h('option', { value: '10' }, '10 utilisations'),
    h('option', { value: '' }, 'Illimité'),
  ]);
  const create = async () => {
    const body = await run(() =>
      api('POST', '/overlay/admin/invites', {
        role: role.value,
        expiresInHours: expiry.value ? Number(expiry.value) : null,
        maxUses: uses.value ? Number(uses.value) : null,
      }),
    );
    if (body && body.invite) {
      copyText(formatCode(body.invite.code), 'Code créé et copié.');
      loadAdmin();
    }
  };
  return h(
    'div',
    null,
    h('div', { class: 'panel row is-wrap' }, role, expiry, uses, h('span', { class: 'spacer' }), h('button', { class: 'btn', onClick: create }, icon('plus'), 'Créer un code')),
    state.invites.length === 0
      ? h('p', { class: 'meta' }, 'Aucune invitation pour le moment.')
      : h(
          'div',
          { class: 'list' },
          state.invites.map((invite) =>
            h(
              'div',
              { class: 'line is-invite' },
              h('span', { class: 'code' }, formatCode(invite.code)),
              h('span', null, h('span', { class: `badge is-${invite.role}` }, ROLE_LABELS[invite.role] || invite.role)),
              h('span', { class: 'sub ellipsis' }, inviteStatus(invite)),
              invite.isActive
                ? h('button', { class: 'btn is-quiet', 'aria-label': `Copier le code ${invite.code}`, title: 'Copier le code', onClick: () => copyText(formatCode(invite.code), 'Code copié.') }, icon('copy'))
                : h('span'),
              invite.isActive ? h('button', { class: 'btn is-quiet is-danger', onClick: () => revokeInvite(invite) }, 'Révoquer') : h('span'),
            ),
          ),
        ),
  );
}

async function revokeInvite(invite) {
  await run(() => api('POST', `/overlay/admin/invites/${encodeURIComponent(invite.code)}/revoke`), 'Invitation révoquée.');
  loadAdmin();
}

async function copyText(text, message) {
  try {
    await navigator.clipboard.writeText(text);
    notify(message);
  } catch {
    notify(`Code : ${text}`);
  }
}

function renderSettings() {
  const guild = (state.info && state.info.guild) || { settings: {} };
  const name = h('input', { class: 'field', id: 'set-name', value: guild.name || '', maxlength: '60' });
  const defaultTime = h('input', { class: 'field', id: 'set-default', type: 'number', min: '0', max: '86400', value: guild.settings.defaultMediaTime ?? '', placeholder: 'Valeur du serveur', style: 'max-width:160px' });
  const maxTime = h('input', { class: 'field', id: 'set-max', type: 'number', min: '0', max: '86400', value: guild.settings.maxMediaTime ?? '', placeholder: 'Sans limite', style: 'max-width:160px' });
  const full = h('input', { type: 'checkbox', id: 'set-full', checked: Boolean(guild.settings.displayMediaFull) });
  const guests = h('input', { type: 'checkbox', id: 'set-guests', checked: guild.guestsCanViewBoard !== false });
  const save = async () => {
    const body = await run(
      () =>
        api('PATCH', '/overlay/admin/settings', {
          name: name.value,
          defaultMediaTime: defaultTime.value === '' ? null : Number(defaultTime.value),
          maxMediaTime: maxTime.value === '' ? null : Number(maxTime.value),
          displayMediaFull: full.checked,
          guestsCanViewBoard: guests.checked,
        }),
      'Réglages enregistrés.',
    );
    if (body) {
      await loadGuild();
      renderView();
    }
  };
  const discord = [
    h('h3', { class: 'section-title' }, 'Discord'),
    h('p', { class: 'meta' }, guild.discordGuildId ? `Guilde liée au serveur Discord ${guild.discordGuildId}.` : guild.standalone ? 'Cette guilde n’est liée à aucun serveur Discord.' : 'Cette guilde est un serveur Discord.'),
  ];
  if (guild.standalone && can('manageSettings')) {
    if (guild.discordGuildId) {
      const unlink = () => {
        confirmDialog('Délier le serveur Discord ?', 'Les commandes de ce serveur Discord ne viseront plus cette guilde.', 'Délier', async () => {
          await run(() => api('DELETE', '/overlay/admin/discord-link'));
          await loadGuild();
          renderView();
        });
      };
      discord.push(h('div', { class: 'row is-wrap' }, h('button', { class: 'btn is-quiet is-danger', onClick: unlink }, 'Délier le serveur Discord')));
    } else {
      const generated = h('div', { class: 'stack', hidden: true });
      const verifyDiscordLink = async () => {
        await loadGuild();
        const linked = Boolean(state.info && state.info.guild && state.info.guild.discordGuildId);
        renderView();
        notify(linked ? 'Serveur Discord lié.' : 'Pas encore lié : saisis la commande dans Discord.', !linked);
      };
      const generate = async () => {
        const body = await run(() => api('POST', '/overlay/admin/discord-link-code'));
        if (!body || !body.code) {
          return;
        }
        const command = `/lier code:${body.code}`;
        generated.replaceChildren(
          h(
            'p',
            { class: 'meta' },
            'Dans le serveur Discord, un administrateur tape /lier code:',
            h('span', { class: 'code' }, body.code),
            ' (ou /link code:',
            h('span', { class: 'code' }, body.code),
            '). Code valable 10 minutes.',
          ),
          h(
            'div',
            { class: 'row is-wrap' },
            h('button', { class: 'btn', onClick: () => copyText(command, 'Commande copiée.') }, 'Copier la commande'),
            h('button', { class: 'btn is-quiet', onClick: verifyDiscordLink }, 'Vérifier la liaison'),
          ),
        );
        generated.hidden = false;
      };
      discord.push(
        h('div', { class: 'row is-wrap' }, h('button', { class: 'btn', onClick: generate }, 'Générer un code de liaison')),
        generated,
      );
    }
  }
  return h(
    'div',
    null,
    h(
      'div',
      { class: 'form' },
      h('label', { for: 'set-name' }, 'Nom de la guilde'),
      name,
      h('label', { for: 'set-default' }, 'Durée par défaut (s)'),
      defaultTime,
      h('p', { class: 'hint' }, 'Durée d’affichage d’une image ou d’un texte sans durée propre.'),
      h('label', { for: 'set-max' }, 'Durée maximale (s)'),
      maxTime,
      h('label', { for: 'set-full' }, 'Affichage'),
      h('label', { class: 'switch' }, full, 'Médias en plein écran'),
      h('label', { for: 'set-guests' }, 'Invités'),
      h('label', { class: 'switch' }, guests, 'Peuvent consulter le board'),
    ),
    h('div', { class: 'row', style: 'margin-top:20px' }, h('button', { class: 'btn', onClick: save }, 'Enregistrer les réglages')),
    discord,
  );
}

// ---------------------------------------------------------------- dialogues ---
function openDialog(build) {
  const root = document.getElementById('dialog-root');
  const close = () => root.replaceChildren();
  const dialog = h('div', { class: 'dialog', role: 'dialog', 'aria-modal': 'true' });
  const backdrop = h('div', { class: 'backdrop', onMousedown: (event) => { if (event.target === backdrop) { close(); } } }, dialog);
  backdrop.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      close();
    }
  });
  dialog.append(...build(close));
  root.replaceChildren(backdrop);
  const first = dialog.querySelector('input:not([type=hidden]), textarea, select, button');
  if (first) {
    first.focus();
  }
}

function resizeAvatar(file) {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => {
      const size = 96;
      const canvas = document.createElement('canvas');
      canvas.width = size;
      canvas.height = size;
      const side = Math.min(image.width, image.height);
      canvas.getContext('2d').drawImage(image, (image.width - side) / 2, (image.height - side) / 2, side, side, 0, 0, size, size);
      URL.revokeObjectURL(image.src);
      resolve(canvas.toDataURL('image/jpeg', 0.82));
    };
    image.onerror = () => resolve(null);
    image.src = URL.createObjectURL(file);
  });
}

function openJoinDialog() {
  openDialog((close) => {
    const known = state.guilds[0] ? state.guilds[0].membership.serverUrl : '';
    const server = h('input', { class: 'field', type: 'url', placeholder: 'https://livechat.exemple.fr', value: known, required: true });
    const code = h('input', { class: 'field code', placeholder: 'K7QX-M2PD', maxlength: '40', autocomplete: 'off', spellcheck: 'false', required: true });
    const name = h('input', { class: 'field', placeholder: 'Ton pseudo', maxlength: '40', value: state.profile.displayName || '' });
    let avatar = state.profile.avatar || null;
    const avatarPreview = h('span', null, avatarNode(name.value || '?', avatar));
    const file = h('input', {
      type: 'file',
      accept: 'image/png,image/jpeg,image/webp',
      hidden: true,
      onChange: async (event) => {
        const picked = event.target.files && event.target.files[0];
        if (picked) {
          avatar = await resizeAvatar(picked);
          avatarPreview.replaceChildren(avatarNode(name.value || '?', avatar));
        }
      },
    });
    const error = h('p', { class: 'error-text', hidden: true });
    const submit = h('button', { class: 'btn is-air', type: 'submit' }, 'Rejoindre');
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          error.hidden = true;
          if (!code.value.trim() || !server.value.trim()) {
            error.textContent = 'Indique le serveur et le code.';
            error.hidden = false;
            return;
          }
          submit.disabled = true;
          submit.textContent = 'Connexion…';
          let response = null;
          try {
            response = await invoke('pairing_consume', {
              payload: { serverUrl: server.value.trim(), code: code.value.trim().replace(/[\s-]/g, ''), displayName: name.value.trim(), avatar },
            });
          } catch (failure) {
            response = { ok: false, message: String(failure) };
          }
          if (response.ok) {
            close();
            notify('Guilde rejointe.');
            applyGuilds(await invoke('guilds_list'));
            return;
          }
          submit.disabled = false;
          submit.textContent = 'Rejoindre';
          error.textContent = /pairing_code_invalid_or_expired/.test(response.message || '')
            ? ERROR_LABELS.pairing_code_invalid_or_expired
            : response.message || 'Impossible de rejoindre la guilde.';
          error.hidden = false;
        },
      },
      h('label', null, 'Serveur', server),
      h('label', null, "Code d'invitation", code),
      h('label', null, 'Pseudo', name),
      h('div', { class: 'row' }, avatarPreview, h('button', { class: 'btn', type: 'button', onClick: () => file.click() }, 'Choisir un avatar'), file),
      error,
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, 'Rejoindre une guilde'), h('p', null, 'Le code reçu d’un admin fixe ton rôle de départ.'), form];
  });
}

function openAddDialog() {
  openDialog((close) => {
    const url = h('input', { class: 'field', type: 'url', placeholder: 'https://…', required: true });
    const title = h('input', { class: 'field', placeholder: 'Chat qui tombe', maxlength: '240' });
    const message = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Texte affiché avec le média' });
    const submit = h('button', { class: 'btn', type: 'submit' }, 'Ajouter au board');
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          submit.disabled = true;
          submit.textContent = 'Téléchargement…';
          const body = await run(() => api('POST', '/overlay/meme-board/items', { url: url.value.trim(), title: title.value, message: message.value }));
          if (body) {
            close();
            notify(body.created ? 'Mème ajouté au board.' : 'Ce média est déjà sur le board.');
            loadItems();
          } else {
            submit.disabled = false;
            submit.textContent = 'Ajouter au board';
          }
        },
      },
      h('label', null, 'Lien du média', url),
      h('label', null, 'Titre (facultatif)', title),
      h('label', null, 'Message (facultatif)', message),
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );
    return [h('h2', null, 'Ajouter un mème'), h('p', null, 'Vidéo, image ou son : TikTok, X, YouTube ou lien direct.'), form];
  });
}

function openEditDialog(item) {
  openDialog((close) => {
    const title = h('input', { class: 'field', maxlength: '240', value: item.title || '' });
    const message = h('textarea', { class: 'field', rows: '3', maxlength: '500', value: item.message || '' });
    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: async (event) => {
          event.preventDefault();
          const body = await run(() => api('PATCH', `/overlay/meme-board/items/${encodeURIComponent(item.id)}`, { title: title.value, message: message.value }), 'Mème modifié.');
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
    return [h('h2', null, 'Modifier le mème'), h('p', null, 'Un champ vide efface la valeur.'), form];
  });
}

function confirmDialog(title, text, actionLabel, action) {
  openDialog((close) => [
    h('h2', null, title),
    h('p', null, text),
    h(
      'div',
      { class: 'actions' },
      h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
      h('button', { class: 'btn is-danger', onClick: async () => { close(); await action(); } }, actionLabel),
    ),
  ]);
}

function openDeleteDialog(item) {
  confirmDialog('Supprimer ce mème ?', `« ${itemTitle(item)} » sera retiré du board de la guilde.`, 'Supprimer', async () => {
    await run(() => api('DELETE', `/overlay/meme-board/items/${encodeURIComponent(item.id)}`), 'Mème supprimé.');
    state.selectedId = null;
    loadItems();
  });
}

function openKickDialog(member) {
  confirmDialog(`Exclure ${member.displayName} ?`, 'Ses overlays sont déconnectés tout de suite. Il lui faudra un nouveau code pour revenir.', 'Exclure', async () => {
    await run(() => api('DELETE', `/overlay/admin/members/${encodeURIComponent(member.id)}`), `${member.displayName} a été exclu.`);
    loadAdmin();
  });
}

function openGuildDialog(guild) {
  openDialog((close) => {
    const member = (state.info && state.info.member) || {};
    const name = h('input', { class: 'field', maxlength: '40', value: member.displayName || guild.membership.displayName || '' });
    const deviceCode = h('p', { class: 'meta', hidden: true });
    const extensionCode = h('p', { class: 'meta', hidden: true });
    const save = async () => {
      const body = await run(() => api('PATCH', '/overlay/profile', { displayName: name.value }), 'Profil enregistré.');
      if (body) {
        close();
        loadGuild();
      }
    };
    const makeDeviceCode = async () => {
      const body = await run(() => api('POST', '/overlay/device-code'));
      if (body && body.invite) {
        deviceCode.replaceChildren('Code valable 10 minutes : ', h('span', { class: 'code' }, formatCode(body.invite.code)));
        deviceCode.hidden = false;
      }
    };
    const makeExtensionCode = async () => {
      const body = await run(() => api('POST', '/overlay/ingest-code'));
      if (!body || !body.code) {
        return;
      }
      extensionCode.replaceChildren(
        'Dans les options de l’extension, saisis l’URL ',
        h('span', { class: 'code' }, body.apiBaseUrl),
        ' et le code ',
        h('span', { class: 'code' }, body.code),
        '. Valable 10 minutes.',
      );
      extensionCode.hidden = false;
      copyText(body.code, 'Code de l’extension copié.');
    };
    const leave = () => {
      close();
      confirmDialog(`Quitter ${guildLabel(guild)} ?`, 'Tu ne recevras plus ses médias. Il te faudra un nouveau code pour revenir.', 'Quitter la guilde', async () => {
        const result = await run(() => invoke('guild_leave', { payload: { guildId: guild.membership.guildId } }));
        if (!result) {
          return;
        }
        if (!result.ok) {
          notify(describeError(result.reason), true);
        }
        applyGuilds(result.guilds);
      });
    };
    return [
      h('h2', null, 'Mon profil'),
      h('p', null, `Dans ${guildLabel(guild)}, en tant que ${(ROLE_LABELS[member.role] || 'membre').toLowerCase()}.`),
      h(
        'div',
        { class: 'stack' },
        h('label', null, 'Pseudo', name),
        h(
          'div',
          { class: 'row is-wrap' },
          h('button', { class: 'btn', onClick: makeDeviceCode }, 'Ajouter un autre appareil'),
          can('play') ? h('button', { class: 'btn', onClick: makeExtensionCode }, 'Appairer l’extension navigateur') : null,
        ),
        deviceCode,
        can('play') ? extensionCode : null,
      ),
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet is-danger', onClick: leave }, 'Quitter la guilde'),
        h('span', { class: 'spacer' }),
        h('button', { class: 'btn is-quiet', onClick: close }, 'Fermer'),
        h('button', { class: 'btn', onClick: save }, 'Enregistrer'),
      ),
    ];
  });
}

// ---------------------------------------------------------------- démarrage ---
listen('overlay://guilds', applyGuilds);

// La file est rafraîchie sur indice WebSocket plutôt que par scrutation : le
// serveur limite toutes les routes à 100 requêtes par 15 minutes et par IP.
let queueHintTimer = null;
listen('overlay://queue-hint', (payload) => {
  if (!payload || payload.guildId !== state.activeGuildId) {
    return;
  }
  clearTimeout(queueHintTimer);
  queueHintTimer = setTimeout(loadQueue, 500);
});

// Repli : un rafraîchissement lent uniquement quand la file est réellement
// visible et au premier plan. Aucun autre appel périodique à loadQueue.
setInterval(() => {
  if (state.tab === 'queue' && document.hasFocus() && !document.hidden) {
    loadQueue();
  }
}, 30000);

// Le focus et le retour de visibilité partagent le même throttle de 15 s.
let lastFocusQueueLoad = 0;
function loadQueueOnFocus() {
  const now = Date.now();
  if (now - lastFocusQueueLoad < 15000) {
    return;
  }
  lastFocusQueueLoad = now;
  loadQueue();
}
window.addEventListener('focus', loadQueueOnFocus);
setInterval(() => {
  if (state.tab === 'queue' && state.queue.playing && !document.hidden) {
    renderView();
  }
}, 1000);
document.addEventListener('visibilitychange', () => {
  if (!document.hidden) {
    loadQueueOnFocus();
  }
});

invoke('guilds_list')
  .then(applyGuilds)
  .catch((error) => {
    renderView();
    notify(String(error.message || error), true);
  });
renderRail();
renderView();
