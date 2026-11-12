/*
 * LiveChat Admin — exploitation du serveur.
 *
 * Provenance : static/admin/app.js, extrait du panneau legacy
 * (server/src/components/admin/adminRoutes.ts), puis réduit à l'exploitation :
 * jeton admin, vue d'ensemble, arrêt de la lecture, révocation des overlays et
 * extensions, vidage du meme board, suppression d'une guilde (avec départ du
 * bot), déliaison Discord, cookie TikTok, création de guilde et émission de
 * codes admin.
 *
 * Script classique chargé en defer ; aucun gestionnaire inline, aucun module ES.
 */

'use strict';

const OVERVIEW_AUTO_REFRESH_MS = 10000;
const GUILD_NAME_MAX_LENGTH = 60;

const state = {
  token: '',
  overview: null,
  adminCode: '',
};

let overviewAutoRefreshRunning = false;

const readToken = () => localStorage.getItem('livechat_admin_token') || '';

const writeToken = (token) => {
  const normalized = String(token || '').trim();

  if (!normalized) {
    localStorage.removeItem('livechat_admin_token');
    return;
  }

  localStorage.setItem('livechat_admin_token', normalized);
};

const setStatus = (message, kind) => {
  const element = document.getElementById('status');

  if (!element) {
    return;
  }

  element.className = 'status ' + (kind || 'muted');
  element.textContent = message;
};

const setInlineStatus = (id, message, kind) => {
  const element = document.getElementById(id);

  if (!element) {
    return;
  }

  element.className = 'status ' + (kind || 'muted');
  element.textContent = message;
};

const createElement = (tagName, className, text) => {
  const element = document.createElement(tagName);

  if (className) {
    element.className = className;
  }

  if (text !== undefined && text !== null) {
    element.textContent = String(text);
  }

  return element;
};

const bytesToHuman = (value) => {
  const numeric = Number(value);
  const bytes = Number.isFinite(numeric) && numeric > 0 ? numeric : 0;
  const kb = 1024;
  const mb = kb * 1024;
  const gb = mb * 1024;

  if (bytes >= gb) {
    return (bytes / gb).toLocaleString('fr-FR', { maximumFractionDigits: 2 }) + ' GB';
  }

  if (bytes >= mb) {
    return (bytes / mb).toLocaleString('fr-FR', { maximumFractionDigits: 2 }) + ' MB';
  }

  if (bytes >= kb) {
    return (bytes / kb).toLocaleString('fr-FR', { maximumFractionDigits: 2 }) + ' KB';
  }

  return bytes.toLocaleString('fr-FR') + ' B';
};

const formatNumber = (value, maximumFractionDigits) => {
  const numeric = Number(value);

  if (!Number.isFinite(numeric)) {
    return '0';
  }

  return numeric.toLocaleString('fr-FR', {
    maximumFractionDigits: maximumFractionDigits === undefined ? 0 : maximumFractionDigits,
  });
};

const formatDateTime = (value) => {
  if (!value) {
    return '';
  }

  const date = new Date(value);

  if (Number.isNaN(date.getTime())) {
    return '';
  }

  return date.toLocaleString('fr-FR', { dateStyle: 'long', timeStyle: 'short' });
};

const formatAdminCode = (value) => {
  const normalized = String(value || '')
    .replace(/[\s-]+/g, '')
    .toUpperCase();

  if (!normalized) {
    return '';
  }

  if (normalized.length <= 4) {
    return normalized;
  }

  return normalized.slice(0, 4) + '-' + normalized.slice(4);
};

const errorLabel = (error) => {
  const code = error instanceof Error && error.message ? error.message : 'request_failed';
  const labels = {
    token_missing: 'le jeton admin est requis.',
    unauthorized: 'le jeton admin est refusé.',
    admin_ui_disabled: "l'interface d'administration est désactivée sur le serveur.",
    admin_local_only: "l'accès est limité à la machine locale.",
    invalid_name: 'le nom de la guilde est invalide.',
    invalid_guild_id: "l'identifiant de guilde est invalide.",
    invalid_payload: 'les données envoyées sont invalides.',
    confirm_guild_id_mismatch: "l'identifiant de confirmation ne correspond pas.",
    invalid_response: 'la réponse du serveur est inattendue.',
    guild_not_found: 'la guilde est introuvable.',
    native_guild_unlink: 'une guilde native ne peut pas être déliée de Discord.',
    not_linked: "la guilde n'est liée à aucun serveur Discord.",
    network_error: 'le serveur est injoignable.',
    request_failed: 'la requête a échoué.',
  };

  return labels[code] || 'erreur inattendue (' + code + ').';
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

  let response;

  try {
    response = await fetch(path, {
      method: options.method || 'GET',
      headers,
      body: hasBody ? JSON.stringify(options.body) : undefined,
    });
  } catch {
    throw new Error('network_error');
  }

  let payload = null;

  try {
    payload = await response.json();
  } catch {
    payload = null;
  }

  if (!response.ok) {
    const code = payload && typeof payload.error === 'string' ? payload.error : 'request_failed';
    throw new Error(code);
  }

  return payload;
};

const metricCard = (label, value, note) => {
  const card = createElement('div', 'metric');
  card.append(createElement('span', 'metric-value', value), createElement('span', 'metric-label', label));

  if (note) {
    card.append(createElement('span', 'metric-note', note));
  }

  return card;
};

const renderMetrics = () => {
  const container = document.getElementById('metrics');
  if (!container) {
    return;
  }

  container.replaceChildren();

  if (!state.overview) {
    const message = state.token
      ? 'Chargement de la vue d’ensemble…'
      : 'Renseignez le jeton admin pour afficher les métriques.';
    container.append(createElement('p', 'empty-state', message));
    return;
  }

  const totals = state.overview.totals || {};
  const cache = state.overview.cache || {};
  const cachePercent = Math.min(100, Math.max(0, Number(cache.usedPercent) || 0));
  const cacheCard = metricCard(
    'Cache média utilisé',
    bytesToHuman(cache.usedBytes),
    formatNumber(cachePercent, 2) + ' % de ' + bytesToHuman(cache.maxBytes),
  );
  const cacheProgress = document.createElement('progress');
  cacheProgress.max = 100;
  cacheProgress.value = cachePercent;
  cacheProgress.setAttribute('aria-label', 'Occupation du cache média');
  cacheCard.append(cacheProgress);

  container.append(
    metricCard('Guildes', formatNumber(totals.guildCount)),
    metricCard('Overlays connectés', formatNumber(totals.overlayConnectedCount)),
    cacheCard,
  );
};

const findGuild = (guildId) => {
  const guilds = state.overview && Array.isArray(state.overview.guilds) ? state.overview.guilds : [];
  return guilds.find((guild) => guild && guild.id === guildId) || null;
};

const guildDiscordInfo = (guild) => {
  const discord = guild && guild.discord && typeof guild.discord === 'object' ? guild.discord : {};
  const kind = discord.kind === 'native' || discord.kind === 'linked' ? discord.kind : 'none';
  const serverId = typeof discord.serverId === 'string' ? discord.serverId : '';

  return { kind, serverId };
};

const discordLeaveStatus = (discord) => {
  const source = discord && typeof discord === 'object' ? discord : {};
  const outcome = typeof source.outcome === 'string' ? source.outcome : 'skipped';

  if (outcome === 'left') {
    return { message: 'Le bot a quitté le serveur Discord.', kind: 'ok' };
  }

  if (outcome === 'not_member') {
    return { message: "Le bot n'était pas sur ce serveur Discord.", kind: 'ok' };
  }

  if (outcome === 'error') {
    const details =
      typeof source.error === 'string' && source.error.trim()
        ? source.error.trim()
        : 'erreur inconnue';

    return {
      message: "Le bot n'a pas pu quitter le serveur : " + details + '.',
      kind: 'warn',
    };
  }

  return { message: '', kind: 'ok' };
};

const playbackStat = (playback) => {
  const stat = createElement('div', 'stat');
  stat.append(createElement('span', 'stat-label', 'Lecture'));

  const grid = createElement('div', 'playback-grid');
  const counters = [
    ['En attente', playback.pending],
    ['En lecture', playback.playing],
    ['Échecs', playback.failed],
    ['Terminés', playback.done],
  ];

  counters.forEach(([label, value]) => {
    const item = createElement('div', 'playback-item');
    item.append(
      createElement('span', 'playback-label', label),
      createElement('strong', 'playback-value', formatNumber(value)),
    );
    grid.append(item);
  });

  stat.append(grid);
  return stat;
};

const buildGuildCard = (guild) => {
  const card = createElement('article', 'guild-card');
  const overlays = guild.overlays || {};
  const board = guild.memeBoard || {};
  const playback = guild.playback || {};
  const discord = guildDiscordInfo(guild);
  const connectedCount = Number(overlays.connectedCount) || 0;
  const totalCount = Number(overlays.total) || 0;
  const boardPercent = Math.min(100, Math.max(0, Number(board.usedPercent) || 0));

  const head = createElement('div', 'guild-head');
  const identity = createElement('div', 'guild-identity');
  identity.append(
    createElement('h3', 'guild-name', guild.name || 'Guilde sans nom'),
    createElement('p', 'guild-id', guild.id || '-'),
  );

  const badges = createElement('div', 'badge-row');
  let discordBadgeText = 'Sans Discord';

  if (discord.kind === 'native') {
    discordBadgeText = 'Discord : serveur ' + discord.serverId;
  } else if (discord.kind === 'linked') {
    discordBadgeText = 'Discord lié : ' + discord.serverId;
  }

  badges.append(
    createElement(
      'span',
      'badge',
      connectedCount > 0 ? formatNumber(connectedCount) + ' overlay(s) connecté(s)' : 'Aucun overlay connecté',
    ),
    createElement('span', 'badge', discordBadgeText),
  );
  head.append(identity, badges);
  card.append(head);

  const stats = createElement('div', 'guild-stats');

  const overlaysStat = createElement('div', 'stat');
  overlaysStat.append(
    createElement('span', 'stat-label', 'Overlays'),
    createElement('strong', 'stat-value', formatNumber(totalCount) + ' total'),
    createElement('span', 'stat-note', formatNumber(connectedCount) + ' connecté(s)'),
  );

  const boardStat = createElement('div', 'stat');
  boardStat.append(
    createElement('span', 'stat-label', 'Meme board'),
    createElement(
      'strong',
      'stat-value',
      bytesToHuman(board.usedBytes) + ' / ' + bytesToHuman(board.maxBytes),
    ),
  );
  const boardProgress = document.createElement('progress');
  boardProgress.max = 100;
  boardProgress.value = boardPercent;
  boardProgress.setAttribute('aria-label', 'Occupation du meme board');
  boardStat.append(
    boardProgress,
    createElement(
      'span',
      'stat-note',
      formatNumber(boardPercent, 2) + ' % · ' + formatNumber(board.itemsCount) + ' élément(s)',
    ),
  );

  stats.append(overlaysStat, boardStat, playbackStat(playback));
  card.append(stats);

  const actions = createElement('div', 'guild-actions');
  const adminCodeButton = createElement('button', 'button', 'Code admin');
  adminCodeButton.type = 'button';
  adminCodeButton.dataset.action = 'admin-code';
  adminCodeButton.dataset.guildId = guild.id || '';

  const stopButton = createElement('button', 'button', 'Stopper la lecture');
  stopButton.type = 'button';
  stopButton.dataset.action = 'stop';
  stopButton.dataset.guildId = guild.id || '';

  const unlinkButton = createElement('button', 'button', 'Délier Discord');
  unlinkButton.type = 'button';
  unlinkButton.dataset.action = 'discord-unlink';
  unlinkButton.dataset.guildId = guild.id || '';

  const revokeButton = createElement('button', 'button danger', 'Purger');
  revokeButton.type = 'button';
  revokeButton.dataset.action = 'revoke-clients';
  revokeButton.dataset.guildId = guild.id || '';

  const clearBoardButton = createElement('button', 'button danger', 'Vider la board');
  clearBoardButton.type = 'button';
  clearBoardButton.dataset.action = 'clear-board';
  clearBoardButton.dataset.guildId = guild.id || '';

  const purgeButton = createElement('button', 'button danger', 'Supprimer');
  purgeButton.type = 'button';
  purgeButton.dataset.action = 'purge';
  purgeButton.dataset.guildId = guild.id || '';

  actions.append(adminCodeButton, stopButton);

  if (discord.kind === 'linked') {
    actions.append(unlinkButton);
  }

  actions.append(revokeButton, clearBoardButton, purgeButton);
  card.append(actions);

  return card;
};

const renderGuilds = () => {
  const container = document.getElementById('guilds');
  if (!container) {
    return;
  }

  container.replaceChildren();

  if (!state.overview) {
    const message = state.token
      ? 'Chargement des guildes…'
      : 'Renseignez le jeton admin pour afficher les guildes.';
    container.append(createElement('p', 'empty-state', message));
    return;
  }

  const guilds = Array.isArray(state.overview.guilds) ? state.overview.guilds : [];

  if (guilds.length === 0) {
    container.append(createElement('p', 'empty-state', 'Aucune guilde pour le moment.'));
    return;
  }

  guilds.forEach((guild) => container.append(buildGuildCard(guild)));
};

const loadOverview = async () => {
  const payload = await api('/admin/api/overview');
  state.overview = payload && typeof payload === 'object' ? payload : {};
  renderMetrics();
  renderGuilds();
};

const loadRuntimeSettings = async () => {
  const payload = await api('/admin/api/runtime-settings');
  const input = document.getElementById('tiktok-cookie-input');
  const tiktokCookie = payload && typeof payload.tiktokCookie === 'string' ? payload.tiktokCookie : '';

  if (input) {
    input.value = tiktokCookie;
  }

  setInlineStatus(
    'tiktok-cookie-status',
    tiktokCookie
      ? 'Cookie TikTok configuré (' + formatNumber(tiktokCookie.length) + ' caractères).'
      : 'Cookie TikTok vide.',
    'muted',
  );
};

const refreshAll = async () => {
  if (!state.token) {
    setStatus('Renseignez le jeton admin puis cliquez sur « Enregistrer le jeton ».', 'warn');
    return;
  }

  setStatus('Chargement des données…', 'muted');

  try {
    await loadOverview();
  } catch (error) {
    setStatus('Impossible de charger la vue d’ensemble : ' + errorLabel(error), 'error');
    return;
  }

  let cookieError = null;

  try {
    await loadRuntimeSettings();
  } catch (error) {
    cookieError = error;
  }

  if (cookieError) {
    setStatus(
      'Vue d’ensemble à jour, mais le cookie TikTok n’a pas pu être chargé : ' + errorLabel(cookieError),
      'error',
    );
    return;
  }

  setStatus(
    'Données à jour. Dernière synchronisation : ' + new Date().toLocaleTimeString('fr-FR') + '.',
    'ok',
  );
};

const autoRefreshOverview = async () => {
  if (!state.token || overviewAutoRefreshRunning) {
    return;
  }

  overviewAutoRefreshRunning = true;

  try {
    await loadOverview();
  } catch (error) {
    if (!(error instanceof Error) || error.message !== 'token_missing') {
      setStatus('Rafraîchissement automatique impossible : ' + errorLabel(error), 'error');
    }
  } finally {
    overviewAutoRefreshRunning = false;
  }
};

const showAdminCode = (details) => {
  const code = String(details.code || '').trim();

  if (!code) {
    return false;
  }

  state.adminCode = code;

  const section = document.getElementById('admin-code-section');
  const context = document.getElementById('admin-code-context');
  const codeElement = document.getElementById('admin-code-value');
  const expiryElement = document.getElementById('admin-code-expiry');

  if (!section || !context || !codeElement || !expiryElement) {
    return false;
  }

  const guildName = String(details.guildName || '').trim() || 'guilde inconnue';
  const guildId = String(details.guildId || '').trim() || '-';
  const prefix =
    details.origin === 'create' ? 'Guilde créée : ' : 'Nouveau code admin pour la guilde ';

  context.textContent = prefix + '« ' + guildName + ' » (' + guildId + ').';
  codeElement.textContent = formatAdminCode(code);

  const expiration = formatDateTime(details.expiresAt);
  expiryElement.textContent = expiration
    ? 'Expire le ' + expiration + '.'
    : 'Aucune date d’expiration fournie.';

  setInlineStatus('copy-code-status', '', 'muted');
  section.hidden = false;
  section.scrollIntoView({ behavior: 'smooth', block: 'nearest' });

  return true;
};

const createGuild = async (event) => {
  event.preventDefault();

  const input = document.getElementById('guild-name');
  const submit = document.getElementById('create-guild-submit');

  if (!(input instanceof HTMLInputElement) || !(submit instanceof HTMLButtonElement)) {
    return;
  }

  const name = String(input.value || '').trim();

  if (!name) {
    setInlineStatus('create-guild-status', 'Le nom de la guilde est obligatoire.', 'warn');
    input.focus();
    return;
  }

  if (name.length > GUILD_NAME_MAX_LENGTH) {
    setInlineStatus(
      'create-guild-status',
      'Le nom de la guilde ne peut pas dépasser 60 caractères.',
      'warn',
    );
    input.focus();
    return;
  }

  submit.disabled = true;
  setInlineStatus('create-guild-status', 'Création de la guilde…', 'muted');

  try {
    const payload = await api('/admin/api/guilds', {
      method: 'POST',
      body: { name },
    });
    const guild = payload && payload.guild && typeof payload.guild === 'object' ? payload.guild : {};
    const invite =
      payload && payload.adminInvite && typeof payload.adminInvite === 'object'
        ? payload.adminInvite
        : {};
    const guildId = typeof guild.id === 'string' ? guild.id : '';
    const guildName =
      typeof guild.name === 'string' && guild.name.trim() ? guild.name.trim() : name;
    const displayed = showAdminCode({
      code: invite.code,
      expiresAt: invite.expiresAt,
      guildName,
      guildId,
      origin: 'create',
    });

    input.value = '';

    if (displayed) {
      setInlineStatus(
        'create-guild-status',
        'Guilde créée. Le code admin est affiché ci-dessous.',
        'ok',
      );
    } else {
      setInlineStatus(
        'create-guild-status',
        'Guilde créée, mais le code admin n’a pas été reçu.',
        'error',
      );
    }

    try {
      await loadOverview();
    } catch (error) {
      setStatus(
        'Guilde créée, mais l’actualisation de la vue d’ensemble a échoué : ' + errorLabel(error),
        'error',
      );
    }
  } catch (error) {
    setInlineStatus('create-guild-status', 'Création impossible : ' + errorLabel(error), 'error');
  } finally {
    submit.disabled = false;
  }
};

const issueAdminCode = async (button, guildId) => {
  const guild = findGuild(guildId);
  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;

  button.disabled = true;
  setStatus('Émission du code admin…', 'muted');

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/admin-code',
      { method: 'POST' },
    );
    const invite =
      payload && payload.adminInvite && typeof payload.adminInvite === 'object'
        ? payload.adminInvite
        : {};
    const responseGuildId =
      payload && typeof payload.guildId === 'string' ? payload.guildId : guildId;
    const displayed = showAdminCode({
      code: invite.code,
      expiresAt: invite.expiresAt,
      guildName,
      guildId: responseGuildId,
      origin: 'admin-code',
    });

    if (!displayed) {
      throw new Error('invalid_response');
    }

    setStatus('Nouveau code admin émis pour la guilde « ' + guildName + ' ».', 'ok');
  } catch (error) {
    setStatus('Impossible d’émettre un code admin : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const stopPlayback = async (button, guildId) => {
  const guild = findGuild(guildId);
  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;

  button.disabled = true;

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/stop',
      { method: 'POST' },
    );
    const released = Number(payload && payload.releasedCount);

    setStatus(
      'Stop playback envoyé pour « ' +
        guildName +
        ' ». Lectures libérées : ' +
        formatNumber(Number.isFinite(released) ? released : 0) +
        '.',
      'ok',
    );

    await loadOverview();
  } catch (error) {
    setStatus('Impossible d’arrêter la lecture : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const revokeClients = async (button, guildId) => {
  const guild = findGuild(guildId);
  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;
  const typed = window.prompt(
    'Révoquer tous les overlays et extensions de la guilde « ' +
      guildName +
      ' » ? Les membres devront rejoindre avec un nouveau code. ' +
      'La meme board et la guilde sont conservées. ' +
      'Retapez l’identifiant de la guilde pour confirmer :',
    '',
  );

  if (typed === null) {
    setStatus('Purge annulée.', 'muted');
    return;
  }

  if (typed.trim() !== guildId) {
    setStatus('Purge annulée : l’identifiant saisi ne correspond pas.', 'warn');
    return;
  }

  button.disabled = true;
  setStatus('Révocation des overlays et extensions en cours…', 'muted');

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/revoke-clients',
      {
        method: 'POST',
        body: { confirmGuildId: guildId },
      },
    );
    const revoked =
      payload && payload.revoked && typeof payload.revoked === 'object' ? payload.revoked : {};
    const count = (value) => {
      const numeric = Number(value);
      return formatNumber(Number.isFinite(numeric) ? numeric : 0);
    };

    setStatus(
      'Guilde « ' +
        guildName +
        ' » purgée : ' +
        count(revoked.overlayClients) +
        ' overlay(s), ' +
        count(revoked.ingestClients) +
        ' extension(s) et ' +
        count(revoked.members) +
        ' membre(s) révoqué(s).',
      'ok',
    );

    await loadOverview();
  } catch (error) {
    setStatus('Purge impossible : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const clearBoard = async (button, guildId) => {
  const guild = findGuild(guildId);
  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;
  const typed = window.prompt(
    'Vider la meme board de la guilde « ' +
      guildName +
      ' » ? Tous les mèmes et leurs médias seront supprimés définitivement. ' +
      'Retapez l’identifiant de la guilde pour confirmer :',
    '',
  );

  if (typed === null) {
    setStatus('Vidage annulé.', 'muted');
    return;
  }

  if (typed.trim() !== guildId) {
    setStatus('Vidage annulé : l’identifiant saisi ne correspond pas.', 'warn');
    return;
  }

  button.disabled = true;
  setStatus('Vidage du meme board en cours…', 'muted');

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/meme-board/clear',
      {
        method: 'POST',
        body: { confirmGuildId: guildId },
      },
    );
    const deleted =
      payload && payload.deleted && typeof payload.deleted === 'object' ? payload.deleted : {};
    const count = (value) => {
      const numeric = Number(value);
      return formatNumber(Number.isFinite(numeric) ? numeric : 0);
    };

    setStatus(
      'Meme board de « ' +
        guildName +
        ' » vidée : ' +
        count(deleted.memeBoardItems) +
        ' mème(s) et ' +
        count(deleted.orphanMediaAssets) +
        ' média(s) supprimé(s).',
      'ok',
    );

    await loadOverview();
  } catch (error) {
    setStatus('Vidage impossible : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const purgeGuild = async (button, guildId) => {
  const guild = findGuild(guildId);
  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;
  const discord = guildDiscordInfo(guild);
  let question =
    'Action irréversible : la guilde « ' +
    guildName +
    ' », ses overlays, extensions, membres, sa meme board et ses médias seront supprimés';

  if (discord.kind !== 'none') {
    question += ', et le bot quittera le serveur Discord ' + discord.serverId;
  }

  question += '. Retapez l’identifiant de la guilde pour confirmer :';

  const typed = window.prompt(question, '');

  if (typed === null) {
    setStatus('Suppression annulée.', 'muted');
    return;
  }

  if (typed.trim() !== guildId) {
    setStatus('Suppression annulée : l’identifiant saisi ne correspond pas.', 'warn');
    return;
  }

  button.disabled = true;
  setStatus('Suppression de la guilde en cours…', 'muted');

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/purge',
      {
        method: 'POST',
        body: { confirmGuildId: guildId, leaveDiscord: discord.kind !== 'none' },
      },
    );
    const deleted =
      payload && payload.deleted && typeof payload.deleted === 'object' ? payload.deleted : {};
    const discordStatus = discordLeaveStatus(payload && payload.discord);
    const count = (value) => {
      const numeric = Number(value);
      return formatNumber(Number.isFinite(numeric) ? numeric : 0);
    };

    let message =
      'Guilde « ' +
      guildName +
      ' » supprimée. Overlays : ' +
      count(deleted.overlayClients) +
      ' · lectures : ' +
      count(deleted.playbackJobs) +
      ' · board : ' +
      count(deleted.memeBoardItems) +
      ' · médias : ' +
      count(deleted.orphanMediaAssets) +
      '.';

    if (discordStatus.message) {
      message += ' ' + discordStatus.message;
    }

    setStatus(message, discordStatus.kind);

    await loadOverview();
  } catch (error) {
    setStatus('Suppression impossible : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const unlinkDiscord = async (button, guildId) => {
  const guild = findGuild(guildId);
  const discord = guildDiscordInfo(guild);

  if (discord.kind !== 'linked') {
    return;
  }

  const guildName = guild && typeof guild.name === 'string' ? guild.name : guildId;
  const confirmed = window.confirm(
    'Délier la guilde « ' +
      guildName +
      ' » du serveur Discord ' +
      discord.serverId +
      ' ? Les commandes de ce serveur ne viseront plus cette guilde.',
  );

  if (!confirmed) {
    setStatus('Déliaison annulée.', 'muted');
    return;
  }

  const leaveDiscord = window.confirm(
    'Faire aussi quitter le bot du serveur Discord ' + discord.serverId + ' ?',
  );

  button.disabled = true;
  setStatus('Déliaison de la guilde en cours…', 'muted');

  try {
    const payload = await api(
      '/admin/api/guilds/' + encodeURIComponent(guildId) + '/discord-unlink',
      {
        method: 'POST',
        body: { leaveDiscord },
      },
    );
    const discordStatus = discordLeaveStatus(payload && payload.discord);
    let message = 'Guilde « ' + guildName + ' » déliée de Discord.';

    if (discordStatus.message) {
      message += ' ' + discordStatus.message;
    }

    setStatus(message, discordStatus.kind);

    await loadOverview();
  } catch (error) {
    setStatus('Déliaison impossible : ' + errorLabel(error), 'error');
  } finally {
    button.disabled = false;
  }
};

const handleActionClick = async (button) => {
  if (button.disabled) {
    return;
  }

  const action = button.dataset.action;
  const guildId = button.dataset.guildId || '';

  if (!action || !guildId) {
    return;
  }

  if (action === 'admin-code') {
    await issueAdminCode(button, guildId);
    return;
  }

  if (action === 'stop') {
    await stopPlayback(button, guildId);
    return;
  }

  if (action === 'discord-unlink') {
    await unlinkDiscord(button, guildId);
    return;
  }

  if (action === 'revoke-clients') {
    await revokeClients(button, guildId);
    return;
  }

  if (action === 'clear-board') {
    await clearBoard(button, guildId);
    return;
  }

  if (action === 'purge') {
    await purgeGuild(button, guildId);
  }
};

const fallbackCopyText = (text) => {
  const helper = document.createElement('textarea');
  helper.value = text;
  helper.setAttribute('readonly', '');
  helper.className = 'copy-helper';
  document.body.append(helper);

  let copied = false;

  try {
    helper.select();
    helper.setSelectionRange(0, helper.value.length);
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  } finally {
    helper.remove();
  }

  return copied;
};

const copyText = async (text) => {
  if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Repli ci-dessous.
    }
  }

  return fallbackCopyText(text);
};

const copyCurrentAdminCode = async () => {
  if (!state.adminCode) {
    return;
  }

  const copied = await copyText(formatAdminCode(state.adminCode));
  setInlineStatus(
    'copy-code-status',
    copied ? 'Code copié dans le presse-papiers.' : '',
    copied ? 'ok' : 'muted',
  );
};

const saveToken = async () => {
  const input = document.getElementById('token-input');
  const token = input instanceof HTMLInputElement ? String(input.value || '').trim() : '';

  state.token = token;
  writeToken(token);

  if (!token) {
    setStatus('Jeton admin effacé.', 'warn');
    return;
  }

  setStatus('Jeton admin enregistré. Chargement des données…', 'muted');
  await refreshAll();
};

const saveTikTokCookie = async (event) => {
  event.preventDefault();

  const input = document.getElementById('tiktok-cookie-input');
  const tiktokCookie = input instanceof HTMLInputElement ? String(input.value || '') : '';

  try {
    await api('/admin/api/runtime-settings', {
      method: 'PATCH',
      body: { tiktokCookie },
    });
    await loadRuntimeSettings();
    setInlineStatus(
      'tiktok-cookie-status',
      tiktokCookie ? 'Cookie TikTok enregistré.' : 'Cookie TikTok vidé.',
      'ok',
    );
    setStatus('Cookie TikTok mis à jour.', 'ok');
  } catch (error) {
    setInlineStatus(
      'tiktok-cookie-status',
      'Mise à jour impossible : ' + errorLabel(error),
      'error',
    );
  }
};

const clearTikTokCookie = async () => {
  try {
    await api('/admin/api/runtime-settings', {
      method: 'PATCH',
      body: { tiktokCookie: '' },
    });
    await loadRuntimeSettings();
    setStatus('Cookie TikTok vidé.', 'ok');
  } catch (error) {
    setInlineStatus(
      'tiktok-cookie-status',
      'Suppression impossible : ' + errorLabel(error),
      'error',
    );
  }
};

const boot = () => {
  const token = readToken();
  state.token = token;

  const tokenInput = document.getElementById('token-input');
  if (tokenInput instanceof HTMLInputElement) {
    tokenInput.value = token;
  }

  document.getElementById('save-token')?.addEventListener('click', () => {
    void saveToken();
  });

  document.getElementById('refresh-overview')?.addEventListener('click', () => {
    void refreshAll();
  });

  document.getElementById('create-guild-form')?.addEventListener('submit', (event) => {
    void createGuild(event);
  });

  document.getElementById('tiktok-cookie-form')?.addEventListener('submit', (event) => {
    void saveTikTokCookie(event);
  });

  document.getElementById('clear-tiktok-cookie')?.addEventListener('click', () => {
    void clearTikTokCookie();
  });

  document.getElementById('copy-admin-code')?.addEventListener('click', () => {
    void copyCurrentAdminCode();
  });

  document.addEventListener('click', (event) => {
    const target = event.target;

    if (!(target instanceof Element)) {
      return;
    }

    const button = target.closest('button[data-action]');

    if (!(button instanceof HTMLButtonElement)) {
      return;
    }

    event.preventDefault();
    void handleActionClick(button);
  });

  window.setInterval(() => {
    void autoRefreshOverview();
  }, OVERVIEW_AUTO_REFRESH_MS);

  if (!token) {
    setStatus('Saisissez le jeton admin puis cliquez sur « Enregistrer le jeton ».', 'warn');
    renderMetrics();
    renderGuilds();
    return;
  }

  void refreshAll();
};

boot();
