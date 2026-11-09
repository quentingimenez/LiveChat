/*
 * LiveChat Admin — logique client extraite du panel legacy.
 *
 * Provenance : server/src/components/admin/adminRoutes.ts, fonction
 * buildAdminPanelHtml(), bloc <script> lignes 1096-2240 (template literal).
 * Extraction mécanique : dé-indentation de 6 espaces et dé-échappement des
 * séquences du template literal (\" -> " et \\' -> \'), aucun autre changement.
 *
 * Modifications volontaires (voir docs/rust/admin-panel.md §4) :
 *  - le handler inline onerror="this.style.display='none'" de l'avatar a été
 *    supprimé du HTML généré et remplacé par un listener délégué 'error'
 *    (capture), afin d'autoriser script-src 'self' sans 'unsafe-inline' ;
 *  - ce fichier est chargé via <script src="app.js" defer> au lieu d'un bloc
 *    inline : même moment d'exécution effectif (DOM déjà parsé).
 * Les appels réseau, chemins, méthodes et payloads sont identiques au legacy.
 */

const state = {
  token: '',
  overview: null,
  pairingItems: [],
  ingestAuthors: [],
};

const OVERVIEW_AUTO_REFRESH_MS = 2000;
const PAIRING_AUTO_REFRESH_MS = 12000;
let isOverviewAutoRefreshRunning = false;
let isPairingAutoRefreshRunning = false;
let autoRefreshBlockedUntil = 0;

const blockAutoRefresh = (durationMs) => {
  const normalizedDuration = Number.isFinite(durationMs) && durationMs > 0 ? Math.floor(durationMs) : 0;
  const nextBlockedUntil = Date.now() + normalizedDuration;
  if (nextBlockedUntil > autoRefreshBlockedUntil) {
    autoRefreshBlockedUntil = nextBlockedUntil;
  }
};

const isEditableControl = (node) => {
  if (!(node instanceof HTMLElement)) {
    return false;
  }

  if (node.isContentEditable) {
    return true;
  }

  if (node instanceof HTMLTextAreaElement) {
    return !node.disabled && !node.readOnly;
  }

  if (node instanceof HTMLSelectElement) {
    return !node.disabled;
  }

  if (node instanceof HTMLInputElement) {
    if (node.disabled || node.readOnly) {
      return false;
    }

    const inputType = String(node.type || '').toLowerCase();
    const nonEditableTypes = ['button', 'submit', 'reset', 'file', 'hidden'];
    return nonEditableTypes.indexOf(inputType) === -1;
  }

  return false;
};

const shouldPauseAutoRefresh = () => {
  const activeElement = document.activeElement;
  if (isEditableControl(activeElement)) {
    return true;
  }

  return Date.now() < autoRefreshBlockedUntil;
};

const escapeHtml = (value) => {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
};

const bytesToHuman = (value) => {
  const bytes = Number.isFinite(value) && value > 0 ? value : 0;
  const kb = 1024;
  const mb = kb * 1024;
  const gb = mb * 1024;

  if (bytes >= gb) {
    return (bytes / gb).toFixed(2) + ' GB';
  }

  if (bytes >= mb) {
    return (bytes / mb).toFixed(2) + ' MB';
  }

  if (bytes >= kb) {
    return (bytes / kb).toFixed(2) + ' KB';
  }

  return bytes + ' B';
};

const toDateTime = (value) => {
  if (!value) {
    return '-';
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return '-';
  }

  return date.toLocaleString('fr-FR');
};

const setStatus = (message, kind) => {
  const element = document.getElementById('status');
  element.className = 'status ' + (kind || 'muted');
  element.textContent = message;
};

const syncTokenInUrl = (token) => {
  const url = new URL(window.location.href);
  const normalized = String(token || '').trim();

  if (normalized) {
    url.searchParams.set('token', normalized);
  } else {
    url.searchParams.delete('token');
  }

  window.history.replaceState({}, '', url.toString());
};

const readToken = () => {
  const queryToken = new URLSearchParams(window.location.search).get('token') || '';

  if (queryToken) {
    localStorage.setItem('livechat_admin_token', queryToken);
    syncTokenInUrl(queryToken);
    return queryToken;
  }

  const storedToken = localStorage.getItem('livechat_admin_token') || '';
  if (storedToken) {
    syncTokenInUrl(storedToken);
  }
  return storedToken;
};

const writeToken = (token) => {
  const normalized = String(token || '').trim();

  if (!normalized) {
    localStorage.removeItem('livechat_admin_token');
    syncTokenInUrl('');
    return;
  }

  localStorage.setItem('livechat_admin_token', normalized);
  syncTokenInUrl(normalized);
};

const api = async (path, options = {}) => {
  const token = state.token;

  if (!token) {
    throw new Error('token_missing');
  }

  const hasBody = options.body !== undefined;
  const headers = {
    Authorization: 'Bearer ' + token,
    ...(options.headers || {}),
  };

  if (hasBody) {
    headers['Content-Type'] = 'application/json';
  }

  const response = await fetch(path, {
    method: options.method || 'GET',
    headers,
    body: hasBody ? JSON.stringify(options.body) : undefined,
  });

  let payload = null;
  try {
    payload = await response.json();
  } catch {
    payload = null;
  }

  if (!response.ok) {
    const errorText = payload && payload.error ? payload.error : 'request_failed';
    throw new Error(errorText);
  }

  return payload;
};

const renderMetrics = () => {
  const container = document.getElementById('metrics');
  const overview = state.overview;

  if (!overview) {
    container.innerHTML = '';
    return;
  }

  const cache = overview.cache;
  const totals = overview.totals;

  container.innerHTML = [
    '<div class="metric"><strong>' + escapeHtml(String(totals.guildCount)) + '</strong><div class="muted">Guilds suivies</div></div>',
    '<div class="metric"><strong>' + escapeHtml(String(totals.overlayConnectedCount)) + '</strong><div class="muted">Overlays connectés</div></div>',
    '<div class="metric"><strong>' + escapeHtml(String(totals.ingestClientCount)) + '</strong><div class="muted">Ingest clients actifs</div></div>',
    '<div class="metric"><strong>' + escapeHtml(String(overview.pairingCodes.active)) + '</strong><div class="muted">Pairing codes actifs</div></div>',
    '<div class="metric"><strong>' + escapeHtml(bytesToHuman(cache.remainingBytes)) + '</strong><div class="muted">Cache restant</div><div class="progress"><span style="width:' + escapeHtml(String(cache.usedPercent)) + '%"></span></div></div>',
  ].join('');
};

const renderGuilds = () => {
  const container = document.getElementById('guilds');
  const overview = state.overview;

  if (!overview || !Array.isArray(overview.guilds) || overview.guilds.length === 0) {
    container.innerHTML = '<div class="muted">Aucune guild disponible.</div>';
    return;
  }

  container.innerHTML = overview.guilds
    .map((guild) => {
      const board = guild.memeBoard;
      const overlays = guild.overlays;
      const ingest = guild.ingest;
      const playback = guild.playback;

      const overlayItems = overlays.clients
        .map((client) => {
          const badgeClass = client.connected ? 'ok' : 'warn';
          const badgeText = client.connected ? 'connecté' : 'hors ligne';
          const modeBadge =
            client.connected && client.sessionMode === 'invite_read_only'
              ? ' <span class="badge info">invité</span>'
              : '';
          const hasAuthorName = typeof client.authorName === 'string' && client.authorName.trim() !== '';
          const displayName = hasAuthorName ? client.authorName.trim() : client.label;
          const authorImage =
            typeof client.authorImage === 'string' && client.authorImage.trim() !== ''
              ? client.authorImage.trim()
              : null;
          const avatar =
            authorImage !== null
              ? '<img class="overlay-avatar" src="' +
                escapeHtml(authorImage) +
                '" alt="Avatar" referrerpolicy="no-referrer" loading="lazy" />'
              : '';
          const deviceLine =
            hasAuthorName && client.authorName.trim() !== client.label
              ? '<div class="muted">device: ' + escapeHtml(client.label) + '</div>'
              : '';
          return (
            '<li class="list-item">' +
            '<div class="overlay-client-head">' +
            avatar +
            '<div><strong>' +
            escapeHtml(displayName) +
            '</strong>' +
            modeBadge +
            ' <span class="badge ' +
            badgeClass +
            '">' +
            badgeText +
            '</span></div></div>' +
            deviceLine +
            '<div class="muted">id: ' +
            escapeHtml(client.id) +
            '</div>' +
            '<div class="muted">lastSeen: ' +
            escapeHtml(toDateTime(client.lastSeenAt)) +
            '</div>' +
            '<button class="action-btn danger" data-action="revoke-overlay" data-client-id="' +
            escapeHtml(client.id) +
            '">Révoquer</button>' +
            '</li>'
          );
        })
        .join('');

      const ingestItems = ingest.clients
        .map((client) => {
          return (
            '<li class="list-item">' +
            '<div><strong>' +
            escapeHtml(client.label) +
            '</strong></div>' +
            '<div class="muted">id: ' +
            escapeHtml(client.id) +
            '</div>' +
            '<div class="muted">lastSeen: ' +
            escapeHtml(toDateTime(client.lastSeenAt)) +
            '</div>' +
            '<button class="action-btn danger" data-action="revoke-ingest" data-client-id="' +
            escapeHtml(client.id) +
            '">Révoquer</button>' +
            '</li>'
          );
        })
        .join('');

      const boardPercent = Number.isFinite(board.usedPercent) ? board.usedPercent : 0;
      const settings = guild.settings || {};
      const settingOverrides = settings.overrides || {};
      const editableGuildName = typeof guild.editableName === 'string' ? guild.editableName : '';
      const defaultEffectiveRaw = settings.defaultMediaTime == null ? '' : String(settings.defaultMediaTime);
      const maxEffectiveRaw = settings.maxMediaTime == null ? '' : String(settings.maxMediaTime);
      const displayEffective = !!settings.displayMediaFull;
      const defaultOverrideRaw =
        settingOverrides.defaultMediaTime == null ? '' : String(settingOverrides.defaultMediaTime);
      const maxOverrideRaw = settingOverrides.maxMediaTime == null ? '' : String(settingOverrides.maxMediaTime);
      const displayOverrideRaw =
        typeof settingOverrides.displayMediaFull === 'boolean' ? String(settingOverrides.displayMediaFull) : '';

      return (
        '<article class="guild-card" data-guild-id="' +
        escapeHtml(guild.id) +
        '">' +
        '<div class="guild-head">' +
        '<div><h3>' +
        escapeHtml(guild.name) +
        '</h3><div class="muted">' +
        escapeHtml(guild.id) +
        '</div></div>' +
        '<div>' +
        (overlays.connectedCount > 0
          ? '<span class="badge ok">' + escapeHtml(String(overlays.connectedCount)) + ' overlay(s) connectés</span>'
          : '<span class="badge warn">Aucun overlay connecté</span>') +
        '</div>' +
        '</div>' +
        '<div class="guild-grid guild-grid-meta">' +
        '<div class="guild-meta">' +
        '<div><strong>Busy Until</strong>: ' +
        escapeHtml(toDateTime(guild.busyUntil)) +
        '</div>' +
        '<div><strong>Jobs</strong>: PENDING ' +
        escapeHtml(String(playback.pending)) +
        ' / PLAYING ' +
        escapeHtml(String(playback.playing)) +
        ' / FAILED ' +
        escapeHtml(String(playback.failed)) +
        ' / DONE ' +
        escapeHtml(String(playback.done)) +
        '</div>' +
        '<div><strong>Meme board</strong>: ' +
        escapeHtml(bytesToHuman(board.usedBytes)) +
        ' / ' +
        escapeHtml(bytesToHuman(board.maxBytes)) +
        '</div>' +
        '<div class="progress"><span style="width:' +
        escapeHtml(String(boardPercent)) +
        '%"></span></div>' +
        '</div>' +
        '<div class="guild-meta">' +
        '<div><strong>Overlays</strong>: ' +
        escapeHtml(String(overlays.total)) +
        ' total</div>' +
        '<div><strong>Ingest clients</strong>: ' +
        escapeHtml(String(ingest.total)) +
        '</div>' +
        '<div><strong>Board items</strong>: ' +
        escapeHtml(String(board.itemsCount)) +
        '</div>' +
        '<div><strong>Cache restant global</strong>: ' +
        escapeHtml(bytesToHuman(overview.cache.remainingBytes)) +
        '</div>' +
        '</div>' +
        '</div>' +
        '<form class="guild-settings" data-action="save-settings" data-guild-id="' +
        escapeHtml(guild.id) +
        '" data-default-effective="' +
        escapeHtml(defaultEffectiveRaw) +
        '" data-default-override="' +
        escapeHtml(defaultOverrideRaw) +
        '" data-max-effective="' +
        escapeHtml(maxEffectiveRaw) +
        '" data-max-override="' +
        escapeHtml(maxOverrideRaw) +
        '" data-display-effective="' +
        escapeHtml(displayEffective ? 'true' : 'false') +
        '" data-display-override="' +
        escapeHtml(displayOverrideRaw) +
        '">' +
        '<label>Default media time (sec)<input type="number" min="1" name="defaultMediaTime" value="' +
        escapeHtml(defaultEffectiveRaw) +
        '" /></label>' +
        '<label>Max media time (sec)<input type="number" min="1" name="maxMediaTime" value="' +
        escapeHtml(maxEffectiveRaw) +
        '" /></label>' +
        '<label class="check"><input type="checkbox" name="displayMediaFull" ' +
        (displayEffective ? 'checked' : '') +
        ' /> Display media full</label>' +
        '<button class="action-btn" type="submit">Sauvegarder réglages</button>' +
        '</form>' +
        '<div class="actions">' +
        '<button class="action-btn warn" data-action="stop-guild" data-guild-id="' +
        escapeHtml(guild.id) +
        '">Stop playback</button>' +
        '<button class="action-btn" data-action="pairing-select" data-guild-id="' +
        escapeHtml(guild.id) +
        '">Filtrer pairing codes</button>' +
        '<button class="action-btn danger" data-action="purge-guild" data-guild-id="' +
        escapeHtml(guild.id) +
        '">Purger guild</button>' +
        '</div>' +
        '<div class="guild-grid guild-grid-clients">' +
        '<div class="list-block"><strong>Overlay clients</strong><ul>' +
        (overlayItems || '<li class="list-item empty">Aucun client</li>') +
        '</ul></div>' +
        '<div class="list-block"><strong>Ingest clients</strong><ul>' +
        (ingestItems || '<li class="list-item empty">Aucun client</li>') +
        '</ul></div>' +
        '</div>' +
        '</article>'
      );
    })
    .join('');
};

const renderPairingGuildFilter = () => {
  const select = document.getElementById('pairing-guild-filter');
  const overview = state.overview;

  if (!overview || !Array.isArray(overview.guilds)) {
    select.innerHTML = '<option value="">Toutes les guilds</option>';
    return;
  }

  const options = ['<option value="">Toutes les guilds</option>']
    .concat(
      overview.guilds.map((guild) => {
        return '<option value="' + escapeHtml(guild.id) + '">' + escapeHtml(guild.name + ' (' + guild.id + ')') + '</option>';
      }),
    )
    .join('');

  const current = select.value;
  select.innerHTML = options;

  if (current) {
    select.value = current;
  }
};

const renderIngestCreateGuildFilter = () => {
  const select = document.getElementById('ingest-create-guild');
  const overview = state.overview;

  if (!overview || !Array.isArray(overview.guilds) || overview.guilds.length === 0) {
    select.innerHTML = '<option value="">Aucune guild disponible</option>';
    return;
  }

  const options = overview.guilds
    .map((guild) => {
      return '<option value="' + escapeHtml(guild.id) + '">' + escapeHtml(guild.name + ' (' + guild.id + ')') + '</option>';
    })
    .join('');

  const current = select.value;
  select.innerHTML = options;

  if (current) {
    select.value = current;
  }
};

const renderIngestCreateAuthorFilter = () => {
  const select = document.getElementById('ingest-create-author');
  const items = Array.isArray(state.ingestAuthors) ? state.ingestAuthors : [];

  if (items.length === 0) {
    select.innerHTML = '<option value="">Aucun auteur en DB</option>';
    return;
  }

  const options = items
    .map((author) => {
      const suffix = author.source === 'mixed' ? 'pairing+ingest' : author.source;
      return (
        '<option value="' +
        escapeHtml(author.discordUserId) +
        '">' +
        escapeHtml(author.displayName + ' (' + author.discordUserId + ') [' + suffix + ']') +
        '</option>'
      );
    })
    .join('');

  const current = select.value;
  select.innerHTML = options;

  if (current) {
    select.value = current;
  }
};

const renderPairingRows = () => {
  const container = document.getElementById('pairing-rows');

  if (!Array.isArray(state.pairingItems) || state.pairingItems.length === 0) {
    container.innerHTML = '<tr><td colspan="6" class="muted">Aucun code</td></tr>';
    return;
  }

  container.innerHTML = state.pairingItems
    .map((item) => {
      return (
        '<tr>' +
        '<td><code>' +
        escapeHtml(item.code) +
        '</code></td>' +
        '<td>' +
        escapeHtml(item.guildId) +
        '</td>' +
        '<td>' +
        escapeHtml(item.authorName || item.createdByDiscordUserId || '-') +
        '</td>' +
        '<td>' +
        escapeHtml(toDateTime(item.expiresAt)) +
        '</td>' +
        '<td>' +
        escapeHtml(item.usedAt ? toDateTime(item.usedAt) : '-') +
        '</td>' +
        '<td>' +
        (item.isActive
          ? '<button class="action-btn danger" data-action="revoke-pairing" data-code="' +
            escapeHtml(item.code) +
            '">Révoquer</button>'
          : '<span class="muted">-</span>') +
        '</td>' +
        '</tr>'
      );
    })
    .join('');
};

const loadOverview = async () => {
  const payload = await api('/admin/api/overview');
  state.overview = payload;
  renderMetrics();
  renderGuilds();
  renderPairingGuildFilter();
  renderIngestCreateGuildFilter();
};

const loadPairingCodes = async () => {
  const guildId = document.getElementById('pairing-guild-filter').value;
  const status = document.getElementById('pairing-status-filter').value;
  const params = new URLSearchParams();

  if (guildId) {
    params.set('guildId', guildId);
  }

  if (status) {
    params.set('status', status);
  }

  const payload = await api('/admin/api/pairing-codes?' + params.toString());
  state.pairingItems = Array.isArray(payload.items) ? payload.items : [];
  renderPairingRows();

  const summary = document.getElementById('pairing-summary');
  summary.className = 'status muted';
  summary.textContent =
    'Codes: ' +
    String(payload.counts.total) +
    ' (actifs: ' +
    String(payload.counts.active) +
    ', expirés/utilisés: ' +
    String(payload.counts.expired) +
    ')';
};

const loadIngestAuthors = async () => {
  const payload = await api('/admin/api/ingest-authors');
  state.ingestAuthors = Array.isArray(payload.items) ? payload.items : [];
  renderIngestCreateAuthorFilter();
};

const loadRuntimeSettings = async () => {
  const payload = await api('/admin/api/runtime-settings');
  const input = document.getElementById('tiktok-cookie-input');
  const runtimeStatus = document.getElementById('tiktok-cookie-status');
  const tiktokCookie = typeof payload.tiktokCookie === 'string' ? payload.tiktokCookie : '';

  input.value = tiktokCookie;
  runtimeStatus.className = 'status muted';
  runtimeStatus.textContent = tiktokCookie
    ? 'TIKTOK_COOKIE configuré (' + String(tiktokCookie.length) + ' caractères).'
    : 'TIKTOK_COOKIE vide.';
};

const copyTextToClipboard = async (value) => {
  const normalized = String(value || '').trim();
  if (!normalized) {
    return false;
  }

  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(normalized);
      return true;
    } catch {
      // fallback below
    }
  }

  const tokenInput = document.getElementById('ingest-create-token');
  tokenInput.focus();
  tokenInput.select();
  tokenInput.setSelectionRange(0, tokenInput.value.length);
  const copied = document.execCommand('copy');
  window.getSelection()?.removeAllRanges();
  return copied;
};

const submitGuildSettings = async (form) => {
  const guildId = form.getAttribute('data-guild-id');
  if (!guildId) {
    return;
  }

  const data = new FormData(form);
  const toOptionalNumber = (value) => {
    const normalized = String(value || '').trim();
    if (!normalized) {
      return null;
    }

    const parsed = Number(normalized);
    return Number.isFinite(parsed) ? parsed : null;
  };
  const toOptionalBool = (value) => {
    const normalized = String(value || '').trim().toLowerCase();
    if (normalized === 'true') {
      return true;
    }

    if (normalized === 'false') {
      return false;
    }

    return null;
  };

  const defaultMediaTimeRaw = String(data.get('defaultMediaTime') || '').trim();
  const maxMediaTimeRaw = String(data.get('maxMediaTime') || '').trim();
  const defaultEffective = toOptionalNumber(form.getAttribute('data-default-effective'));
  const maxEffective = toOptionalNumber(form.getAttribute('data-max-effective'));
  const currentDefaultOverride = toOptionalNumber(form.getAttribute('data-default-override'));
  const currentMaxOverride = toOptionalNumber(form.getAttribute('data-max-override'));
  const displayEffective = toOptionalBool(form.getAttribute('data-display-effective')) === true;
  const currentDisplayOverride = toOptionalBool(form.getAttribute('data-display-override'));
  const selectedDisplayValue = data.get('displayMediaFull') === 'on';

  let nextDefaultOverride = defaultMediaTimeRaw ? Number(defaultMediaTimeRaw) : null;
  let nextMaxOverride = maxMediaTimeRaw ? Number(maxMediaTimeRaw) : null;

  if (
    currentDefaultOverride === null &&
    nextDefaultOverride !== null &&
    defaultEffective !== null &&
    nextDefaultOverride === defaultEffective
  ) {
    nextDefaultOverride = null;
  }

  if (currentMaxOverride === null && nextMaxOverride !== null && maxEffective !== null && nextMaxOverride === maxEffective) {
    nextMaxOverride = null;
  }

  let nextDisplayOverride = selectedDisplayValue;
  if (currentDisplayOverride === null && selectedDisplayValue === displayEffective) {
    nextDisplayOverride = null;
  }

  const payload = {};

  if (nextDefaultOverride !== currentDefaultOverride) {
    payload.defaultMediaTime = nextDefaultOverride;
  }

  if (nextMaxOverride !== currentMaxOverride) {
    payload.maxMediaTime = nextMaxOverride;
  }

  if (nextDisplayOverride !== null && nextDisplayOverride !== currentDisplayOverride) {
    payload.displayMediaFull = nextDisplayOverride;
  }

  if (Object.keys(payload).length === 0) {
    setStatus('Aucun changement de réglages.', 'muted');
    return;
  }

  await api('/admin/api/guilds/' + encodeURIComponent(guildId) + '/settings', {
    method: 'PATCH',
    body: payload,
  });

  setStatus('Réglages guild sauvegardés.', 'ok');
  await loadOverview();
};


const handleActionClick = async (button) => {
  const action = button.getAttribute('data-action');

  if (!action) {
    return;
  }

  if (action === 'stop-guild') {
    const guildId = button.getAttribute('data-guild-id');
    if (!guildId) {
      return;
    }

    await api('/admin/api/guilds/' + encodeURIComponent(guildId) + '/stop', {
      method: 'POST',
    });

    setStatus('Stop playback envoyé pour la guild ' + guildId + '.', 'warn');
    await loadOverview();
    return;
  }

  if (action === 'revoke-overlay') {
    const clientId = button.getAttribute('data-client-id');
    if (!clientId) {
      return;
    }

    await api('/admin/api/overlay-clients/' + encodeURIComponent(clientId) + '/revoke', {
      method: 'POST',
    });

    setStatus('Overlay client révoqué: ' + clientId, 'warn');
    await loadOverview();
    return;
  }

  if (action === 'revoke-ingest') {
    const clientId = button.getAttribute('data-client-id');
    if (!clientId) {
      return;
    }

    await api('/admin/api/ingest-clients/' + encodeURIComponent(clientId) + '/revoke', {
      method: 'POST',
    });

    setStatus('Ingest client révoqué: ' + clientId, 'warn');
    await loadOverview();
    return;
  }

  if (action === 'revoke-pairing') {
    const code = button.getAttribute('data-code');
    if (!code) {
      return;
    }

    await api('/admin/api/pairing-codes/' + encodeURIComponent(code) + '/revoke', {
      method: 'POST',
    });

    setStatus('Pairing code révoqué: ' + code, 'warn');
    await loadPairingCodes();
    return;
  }

  if (action === 'pairing-select') {
    const guildId = button.getAttribute('data-guild-id') || '';
    const guildSelect = document.getElementById('pairing-guild-filter');
    guildSelect.value = guildId;
    await loadPairingCodes();
    setStatus('Filtre pairing positionné sur la guild ' + guildId + '.', 'ok');
    return;
  }

  if (action === 'purge-guild') {
    const guildId = button.getAttribute('data-guild-id');
    if (!guildId) {
      return;
    }

    const typedGuildId = window.prompt(
      'Action irréversible. Tapez l\'ID exact de la guild pour confirmer la purge complète.',
      '',
    );
    if (typedGuildId !== guildId) {
      setStatus('Purge annulée (confirmation invalide).', 'warn');
      return;
    }

    const result = await api('/admin/api/guilds/' + encodeURIComponent(guildId) + '/purge', {
      method: 'POST',
      body: {
        confirmGuildId: typedGuildId,
        removeOrphanMedia: true,
      },
    });

    const deleted = result && typeof result === 'object' ? result.deleted || {} : {};
    const summary =
      'Guild purgée ' +
      guildId +
      ' | overlays=' +
      String(deleted.overlayClients || 0) +
      ', ingest=' +
      String(deleted.ingestClients || 0) +
      ', pairing=' +
      String(deleted.pairingCodes || 0) +
      ', jobs=' +
      String(deleted.playbackJobs || 0) +
      ', board=' +
      String(deleted.memeBoardItems || 0) +
      ', assets=' +
      String(deleted.orphanMediaAssets || 0);
    setStatus(summary, 'warn');
    await refreshAll();
    return;
  }
};

const refreshAll = async () => {
  try {
    setStatus('Chargement des données admin...', 'muted');
    await loadOverview();
    await loadIngestAuthors();
    await loadPairingCodes();
    await loadRuntimeSettings();
    setStatus('Données à jour. Dernière synchro: ' + new Date().toLocaleTimeString('fr-FR'), 'ok');
  } catch (error) {
    const code = error instanceof Error ? error.message : 'request_failed';
    if (code === 'token_missing') {
      setStatus('Token manquant. Renseignez ADMIN_UI_TOKEN.', 'warn');
    } else {
      setStatus('Erreur admin: ' + code, 'error');
    }
  }
};

const autoRefreshOverview = async () => {
  if (isOverviewAutoRefreshRunning) {
    return;
  }

  if (shouldPauseAutoRefresh()) {
    return;
  }

  isOverviewAutoRefreshRunning = true;
  try {
    await loadOverview();
  } catch (error) {
    const code = error instanceof Error ? error.message : 'request_failed';
    if (code !== 'token_missing') {
      setStatus('Erreur refresh overview: ' + code, 'error');
    }
  } finally {
    isOverviewAutoRefreshRunning = false;
  }
};

const autoRefreshPairingCodes = async () => {
  if (isPairingAutoRefreshRunning) {
    return;
  }

  if (shouldPauseAutoRefresh()) {
    return;
  }

  isPairingAutoRefreshRunning = true;
  try {
    await loadPairingCodes();
  } catch (error) {
    const code = error instanceof Error ? error.message : 'request_failed';
    if (code !== 'token_missing') {
      setStatus('Erreur refresh pairing: ' + code, 'error');
    }
  } finally {
    isPairingAutoRefreshRunning = false;
  }
};

document.addEventListener(
  'error',
  (event) => {
    const target = event.target;

    if (target instanceof HTMLImageElement && target.classList.contains('overlay-avatar')) {
      target.setAttribute('hidden', '');
    }
  },
  true,
);

document.addEventListener(
  'focusin',
  (event) => {
    if (isEditableControl(event.target)) {
      blockAutoRefresh(4000);
    }
  },
  true,
);

document.addEventListener(
  'input',
  (event) => {
    if (isEditableControl(event.target)) {
      blockAutoRefresh(4000);
    }
  },
  true,
);

document.addEventListener(
  'change',
  (event) => {
    if (isEditableControl(event.target)) {
      blockAutoRefresh(4000);
    }
  },
  true,
);

document.addEventListener(
  'focusout',
  (event) => {
    if (isEditableControl(event.target)) {
      blockAutoRefresh(1500);
    }
  },
  true,
);

document.getElementById('save-token').addEventListener('click', async () => {
  const tokenInput = document.getElementById('token-input');
  const token = String(tokenInput.value || '').trim();

  state.token = token;
  writeToken(token);

  if (!token) {
    setStatus('Token vidé.', 'warn');
    return;
  }

  await refreshAll();
});

document.getElementById('refresh-overview').addEventListener('click', async () => {
  await refreshAll();
});

document.getElementById('save-tiktok-cookie').addEventListener('click', async () => {
  try {
    const input = document.getElementById('tiktok-cookie-input');
    const tiktokCookie = String(input.value || '');

    await api('/admin/api/runtime-settings', {
      method: 'PATCH',
      body: {
        tiktokCookie,
      },
    });

    await loadRuntimeSettings();
    setStatus('TIKTOK_COOKIE mis à jour.', 'ok');
  } catch (error) {
    setStatus('Erreur TIKTOK_COOKIE: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

document.getElementById('clear-tiktok-cookie').addEventListener('click', async () => {
  try {
    await api('/admin/api/runtime-settings', {
      method: 'PATCH',
      body: {
        tiktokCookie: '',
      },
    });

    await loadRuntimeSettings();
    setStatus('TIKTOK_COOKIE vidé.', 'warn');
  } catch (error) {
    setStatus('Erreur TIKTOK_COOKIE: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

document.getElementById('ingest-create-submit').addEventListener('click', async () => {
  try {
    const guildSelect = document.getElementById('ingest-create-guild');
    const authorSelect = document.getElementById('ingest-create-author');
    const labelInput = document.getElementById('ingest-create-label');
    const guildId = String(guildSelect.value || '').trim();
    const authorDiscordUserId = String(authorSelect.value || '').trim();
    const label = String(labelInput.value || '').trim();

    if (!guildId || !authorDiscordUserId) {
      setStatus('Sélectionnez une guild et un auteur.', 'warn');
      return;
    }

    if (!label) {
      setStatus('Le label ingest est obligatoire.', 'warn');
      return;
    }

    const payload = {
      guildId,
      authorDiscordUserId,
      label,
    };

    const created = await api('/admin/api/ingest-clients', {
      method: 'POST',
      body: payload,
    });

    const tokenInput = document.getElementById('ingest-create-token');
    tokenInput.value = typeof created.ingestApiToken === 'string' ? created.ingestApiToken : '';

    const ingestStatus = document.getElementById('ingest-create-status');
    ingestStatus.className = 'status ok';
    ingestStatus.textContent =
      'Client ingest créé: ' +
      String(created?.client?.label || '-') +
      ' (' +
      String(created?.client?.id || '-') +
      ')';

    await loadOverview();
    await loadIngestAuthors();
    setStatus('Client ingest créé et token généré.', 'ok');
  } catch (error) {
    setStatus('Erreur création ingest client: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

document.getElementById('ingest-create-copy-token').addEventListener('click', async () => {
  try {
    const tokenInput = document.getElementById('ingest-create-token');
    const value = String(tokenInput.value || '').trim();

    if (!value) {
      setStatus('Aucun token à copier.', 'warn');
      return;
    }

    const copied = await copyTextToClipboard(value);
    if (!copied) {
      setStatus('Impossible de copier le token ingest.', 'error');
      return;
    }

    setStatus('Token ingest copié dans le presse-papiers.', 'ok');
  } catch (error) {
    setStatus('Impossible de copier le token ingest.', 'error');
  }
});

document.getElementById('pairing-load').addEventListener('click', async () => {
  try {
    await loadPairingCodes();
  } catch (error) {
    setStatus('Erreur pairing codes: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

document.getElementById('pairing-purge').addEventListener('click', async () => {
  try {
    const guildId = document.getElementById('pairing-guild-filter').value;
    const params = new URLSearchParams();
    if (guildId) {
      params.set('guildId', guildId);
    }

    await api('/admin/api/pairing-codes/expired?' + params.toString(), {
      method: 'DELETE',
    });

    setStatus('Pairing codes expirés purgés.', 'warn');
    await loadPairingCodes();
    await loadOverview();
  } catch (error) {
    setStatus('Erreur purge pairing codes: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

document.addEventListener('submit', async (event) => {
  const form = event.target;

  if (!(form instanceof HTMLFormElement)) {
    return;
  }

  const action = form.getAttribute('data-action');
  if (action === 'save-settings') {
    event.preventDefault();

    try {
      await submitGuildSettings(form);
    } catch (error) {
      setStatus('Erreur sauvegarde settings: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
    }
    return;
  }

});

document.addEventListener('click', async (event) => {
  const target = event.target;

  if (!(target instanceof HTMLElement)) {
    return;
  }

  const button = target.closest('button[data-action]');

  if (!(button instanceof HTMLButtonElement)) {
    return;
  }

  event.preventDefault();

  try {
    await handleActionClick(button);
  } catch (error) {
    setStatus('Erreur action admin: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
  }
});

const boot = async () => {
  const token = readToken();
  state.token = token;

  const tokenInput = document.getElementById('token-input');
  tokenInput.value = token;

  if (!token) {
    setStatus('Saisissez ADMIN_UI_TOKEN puis cliquez "Enregistrer token".', 'warn');
    return;
  }

  await refreshAll();
  setInterval(autoRefreshOverview, OVERVIEW_AUTO_REFRESH_MS);
  setInterval(autoRefreshPairingCodes, PAIRING_AUTO_REFRESH_MS);
};

boot();
