// Harnais de développement : simule le shell Tauri pour afficher l'application
// principale (`web/board.html`) dans un navigateur ordinaire, sans serveur.
// Chargé par `dev/board-mock.html` avant `web/app/main.js`. Jamais embarqué dans
// l'application (le dossier `dev/` est hors de `frontendDist`).
//
// Options de l'URL : ?role=admin|member|guest, ?empty (aucune guilde),
// ?removed (un retrait de guilde en attente au démarrage), ?settings (la page
// Paramètres est demandée au démarrage via board_take_pending_view),
// ?update=ready|downloading (état de l'updater au démarrage, pour voir le
// bandeau de mise à jour de la board).
// Depuis la console : `__mock.emit(event, payload)`,
// `__mock.removeGuild(guildId, reason)`, `__mock.finishPlaying()`,
// `__mock.reconnect(guildId)` (coupure puis retour du WebSocket),
// `__mock.restartServer()` (perd les conversions en cours),
// `__mock.openSettings()` (équivalent de l'entrée tray « Paramètres… »),
// `__mock.settings` (réglages simulés).
// Paramètres : settings_get/settings_set (validation des écrans, volumes,
// pause, démarrage automatique, raccourcis, disposition et profil),
// settings_identify_displays, settings_test_media (overlay_busy si un média
// passe), settings_preview_layout (dernier brouillon dans
// `__mock.previewLayouts`), settings_placement_start / settings_placement_cancel
// (`__mock.finishPlacement(zone)` simule la validation sur l'overlay, `null`
// l'annulation), overlay_check_updates et board_take_pending_view.
// Chaque changement émet `overlay://device-settings` avec une copie profonde.
// Ajouts (lien ou fichier) : le serveur répond 202 puis envoie le résultat
// `overlay://media-ingestion` et `overlay://board-changed`. Selon le nom du
// fichier ou le lien : « big » → 413 pendant l'envoi, « fail » → conversion
// en échec, « dup » → déjà sur le board, « race » → résultat reçu avant le 202,
// « slow » → conversion qui ne finit qu'au bout de 20 s.
// « Jouer une fois » (playOnce) : le média devient un job de lecture, sans
// item de board ni `board.changed`, et le résultat porte `jobId`.
// « Sad violin » n'est pas prêt (media_not_ready).
// Éditeur : `?media` sert une vraie vidéo et un vrai son (dev/fixtures) ;
// POST …/items/{id}/edit et `replaceItemId` à l'envoi créent une copie ou
// remplacent le média de l'item, comme le serveur.
// Appairage : le code NOPSEUDO sans pseudo renvoie display_name_required.
// Codes d'appairage : GET /overlay/admin/pairing-codes sert un code Discord
// actif et un code d'extension expiré ; la révocation et la purge modifient
// cette liste en mémoire. Les membres portent discordUserId.
// Appareils : GET/PATCH/DELETE /overlay/admin/devices et
// POST /overlay/admin/device-codes servent quatre appareils sur trois membres
// (un hors ligne, un obsolète), avec renommage, réglages et révocation en
// mémoire ; l'appareil « toi » refuse sa propre révocation.
// Les médias ont des formats variés (portrait, carré, paysage) pour la
// mosaïque ; « Nope » est une image portrait (1080 × 1920). La mise en pause ferme la
// connexion comme le shell (disabled, puis connecting → connected au retour)
// et le membre « toi » passe hors ligne. `__mock.requests` liste les appels
// guild_api ({ method, path }).
// Administration de guilde : GET /overlay/guild porte `mediaBoard` (occupation,
// quota propre, limite serveur) et `playback` ; PATCH /overlay/admin/settings
// accepte `boardQuotaMb` (refus au-dessus de la limite serveur) ;
// POST /overlay/admin/media-board/clear vide le board, arrête la lecture et
// demande le nom exact de la guilde ; GET /overlay/admin/audit sert un journal
// borné à la guilde ; GET /overlay/queue/history sert les lectures terminées et
// en échec ; les jobs de la file portent `origin` (`board` ou `direct`).
(function () {
  'use strict';

  const role = new URLSearchParams(location.search).get('role') || 'admin';
  const empty = new URLSearchParams(location.search).has('empty');
  const isAdmin = role === 'admin';
  const isGuest = role === 'guest';

  const membership = (guildId, guildName, extra) =>
    Object.assign(
      { serverUrl: 'https://livechat.exemple.fr', guildId, guildName, clientId: 'c-' + guildId, memberId: 'm-' + guildId, displayName: 'Quentin', role, guestMode: isGuest, muted: false, active: false },
      extra || {},
    );
  const connection = (status) => ({ status, reason: null, serverVersion: '3.0.0', minVersion: null, guildId: null, clientId: null, sessionMode: 'normal', peers: [], lastError: null });

  // Guild icons: real image files from dev/fixtures (Discord-style tiles), so
  // the rail shows actual pictures like the paired Discord guilds.
  const guildIconUrl = (name) => new URL('./fixtures/' + name, location.href).href;

  const guilds = {
    activeGuildId: empty ? null : '111',
    enabled: true,
    profile: { displayName: 'Quentin', avatar: null },
    guilds: empty
      ? []
      : [
          { membership: membership('111', 'Les Sardines', { active: true, guildIconUrl: guildIconUrl('guild-111.png') }), connection: connection('connected'), statusLabel: 'connecté', playing: true },
          { membership: membership('222', 'GG Squad', { guildIconUrl: guildIconUrl('guild-222.png') }), connection: connection('reconnecting'), statusLabel: 'reconnexion…', playing: false },
          { membership: membership('333', 'TV du salon', { muted: true, guildIconUrl: guildIconUrl('guild-333.png') }), connection: connection('disabled'), statusLabel: 'désactivé', playing: false },
          // An icon that fails to load: the rail falls back to the initials.
          { membership: membership('444', 'L’Escale 🔱', { guildIconUrl: 'https://cdn.discordapp.com/icons/0/missing.png' }), connection: connection('connected'), statusLabel: 'connecté', playing: false },
        ],
  };

  const now = Date.now();

  // `?update=ready|downloading` : état de l'updater au démarrage, pour prévoir
  // le bandeau de mise à jour de la board (« Plus tard » et l'erreur
  // d'installation sont testables aussi).
  const UPDATE_VERSION = '3.2.0';

  function initialUpdate() {
    const flag = new URLSearchParams(location.search).get('update');
    if (flag !== 'ready' && flag !== 'downloading') {
      return { state: 'idle', currentVersion: '3.1.0', availableVersion: null, message: 'Les mises à jour sont vérifiées automatiquement.' };
    }
    return {
      state: flag,
      currentVersion: '3.1.0',
      availableVersion: UPDATE_VERSION,
      message: flag === 'ready' ? `La version ${UPDATE_VERSION} est prête à être installée.` : `Téléchargement de la version ${UPDATE_VERSION}…`,
    };
  }

  const settings = {
    version: '3.1.0',
    update: initialUpdate(),
    displays: [
      { index: 0, key: 'display-0', label: 'Écran 1 — 2560 × 1440 (principal)', width: 2560, height: 1440, primary: true },
      { index: 1, key: 'display-1', label: 'Écran 2 — 1920 × 1080', width: 1920, height: 1080, primary: false },
    ],
    selectedDisplayKey: null,
    volume: 0.8,
    guilds: Object.fromEntries(
      guilds.guilds.map((guild) => [guild.membership.guildId, { volume: 1, muted: false, mediaMuted: false, showText: null, layout: null }]),
    ),
    generalMute: false,
    mediaMuted: false,
    showText: true,
    hoverDetails: true,
    autoStart: false,
    shortcuts: { stop: 'CommandOrControl+Shift+S', stopLocal: null, muteAll: null },
    layout: { zone: { x: 0, y: 0, width: 1, height: 1 }, align: 'center' },
    placing: false,
    profile: { displayName: 'Quentin', avatar: null },
  };
  let pendingView = new URLSearchParams(location.search).has('settings') ? 'settings' : null;

  // Media sizes (width, height): portrait, square and wide shapes for the
  // masonry grid and the detail panel. Others are 1280 × 720.
  const SIZES = { 1: [1080, 1080], 3: [1080, 1920], 4: [1200, 800], 5: [720, 1280], 6: [2400, 1000], 8: [1080, 1920] };
  const sizeOf = (index) => SIZES[index] || [1280, 720];
  // `?media` : every card serves a real fixture from dev/fixtures (frames cut
  // from sample.mp4, JPEG stills and audio artwork), so thumbnails, the editor
  // and the README screenshots render like the real board.
  const realMedia = new URLSearchParams(location.search).has('media');
  const REAL_MEDIA = { 0: 'sample.mp4', 1: 'media-1.jpg', 2: 'sample.mp3', 3: 'seg-3.mp4', 4: 'media-4.jpg', 5: 'seg-5.mp4', 6: 'seg-6.mp4', 8: 'media-8.jpg' };
  const titles = ['Chat qui tombe', 'Bruh', 'Vine boom', 'Directed by Robert B. Weide', 'Stonks', 'Ah shit, here we go again', null, 'Fanfare de la victoire totale et définitive', 'Nope', 'Sad violin'];
  const kinds = ['video', 'image', 'audio', 'video', 'image', 'video', 'video', 'audio', 'image', 'audio'];
  const items = titles.map((title, index) => ({
    id: 'item-' + index,
    guildId: '111',
    mediaAssetId: 'a' + index,
    mediaUrl: svg(index),
    mediaUrlExpiresAt: now + 3600000,
    title,
    message: index === 0 ? 'aïe' : null,
    createdByName: index % 2 ? 'Léa' : 'Quentin',
    createdByMemberId: index % 2 ? 'm-lea' : 'm-111',
    canEdit: isAdmin || (!isGuest && index % 2 === 0),
    createdAt: new Date(Date.now() - index * 86400000).toISOString(),
    media: { id: 'a' + index, kind: kinds[index], mime: 'x', durationSec: kinds[index] === 'image' ? null : 3 + index * 7, width: sizeOf(index)[0], height: sizeOf(index)[1], isVertical: sizeOf(index)[1] > sizeOf(index)[0], sizeBytes: 420000 * (index + 1), sourceUrl: index === 2 ? 'upload:vine-boom.mp3' : index === 3 ? 'https://youtu.be/x' + index : 'https://www.tiktok.com/@x/video/' + index },
    favorite: index === 1 || index === 7,
    // « Fanfare… » and « Vine boom » (audio) have an embedded cover.
    artwork: index === 7 || (realMedia && index === 2)
      ? { url: realMedia ? new URL(index === 7 ? './fixtures/art-7.jpg' : './fixtures/art-2.jpg', location.href).href : placeholder(4), width: 480, height: 480 }
      : null,
    // « Directed by… » (a YouTube link) has its platform thumbnail; with
    // `?media` its real video (seg-3.mp4) is shown instead.
    thumbnail: index === 3 && !realMedia ? { url: placeholder(5), width: 480, height: 360 } : null,
    artworkOnOverlay: false,
    displayDurationSec: null,
    displayScalePercent: null,
  }));

  const queue = {
    playing: { id: 'j0', status: 'playing', title: 'Chat qui tombe', text: null, authorName: 'Léa', durationSec: 30, kind: 'video', origin: 'board', startedAt: new Date(now - 12000).toISOString(), submittedAt: new Date(now - 20000).toISOString() },
    pending: [
      { id: 'j1', status: 'pending', title: 'Stonks', authorName: 'Max', durationSec: 10, kind: 'image', origin: 'board' },
      { id: 'j2', status: 'pending', title: null, text: 'gg les gars', authorName: 'Léa', durationSec: 5, kind: null, origin: 'direct' },
      { id: 'j3', status: 'pending', title: 'Directed by Robert B. Weide', authorName: 'Invité-42', durationSec: 8, kind: 'video', origin: 'board' },
    ],
  };
  // `GET /overlay/queue/history` : lectures terminées et échecs, du plus récent
  // au plus ancien. Les lectures finies pendant la session s'y ajoutent.
  const queueHistory = [
    { id: 'h1', status: 'done', title: 'Bruh', text: null, authorName: 'Léa', durationSec: 4, kind: 'image', origin: 'board', finishedAt: new Date(now - 12 * 60000).toISOString() },
    { id: 'h2', status: 'failed', title: null, text: 'gg les gars', authorName: 'Max', durationSec: 5, kind: null, origin: 'direct', finishedAt: new Date(now - 48 * 60000).toISOString() },
    { id: 'h3', status: 'done', title: 'Vine boom', text: null, authorName: 'Quentin', durationSec: 3, kind: 'audio', origin: 'board', finishedAt: new Date(now - 3 * 3600000).toISOString() },
  ];

  // Guild settings edited from « Réglages » (`PATCH /overlay/admin/settings`).
  let guildName = 'Les Sardines';
  let guestsCanViewBoard = true;
  const guildSettings = { defaultMediaTime: 30, maxMediaTime: null, displayMediaFull: false };
  // Quota propre de la guilde (`null` = limite du serveur).
  const SERVER_BOARD_MAX_BYTES = 5 * 1024 * 1024 * 1024;
  let boardQuotaMb = null;

  function mediaBoardPayload() {
    const usedBytes = items.reduce((sum, item) => sum + (Number(item.media && item.media.sizeBytes) || 0), 0);
    const maxBytes = boardQuotaMb ? boardQuotaMb * 1024 * 1024 : SERVER_BOARD_MAX_BYTES;
    return {
      itemsCount: items.length,
      usedBytes,
      maxBytes,
      remainingBytes: Math.max(0, maxBytes - usedBytes),
      quotaMb: boardQuotaMb,
      serverMaxBytes: SERVER_BOARD_MAX_BYTES,
    };
  }

  // `GET /overlay/admin/audit` : journal d'administration de la guilde, tenu
  // par le panneau serveur et par l'overlay (`actor.method === 'overlay'`).
  const auditEntries = [
    { id: 6, at: new Date(now - 60000).toISOString(), actor: { method: 'overlay', name: 'Quentin' }, action: 'overlay.member_update', method: 'PATCH', path: '/overlay/admin/members/m-lea', status: 200, guildId: '111', clientIp: null },
    { id: 5, at: new Date(now - 8 * 60000).toISOString(), actor: { method: 'secret', name: 'Owner' }, action: 'guild.settings', method: 'PATCH', path: '/admin/api/guilds/111/settings', status: 200, guildId: '111', clientIp: null },
    { id: 4, at: new Date(now - 2 * 3600000).toISOString(), actor: { method: 'discord', name: 'Admin Discord' }, action: 'guild.board_clear', method: 'POST', path: '/admin/api/guilds/111/media-board/clear', status: 200, guildId: '111', clientIp: null },
    { id: 3, at: new Date(now - 26 * 3600000).toISOString(), actor: { method: 'secret', name: 'Owner' }, action: 'guild.member_ban', method: 'POST', path: '/admin/api/guilds/111/members/m-42/ban', status: 200, guildId: '111', clientIp: null },
    { id: 2, at: new Date(now - 3 * 86400000).toISOString(), actor: { method: 'secret', name: 'Owner' }, action: 'guild.invite_create', method: 'POST', path: '/admin/api/guilds/111/invites', status: 200, guildId: '111', clientIp: null },
  ];

  const members = [
    { id: 'm-111', displayName: 'Quentin', avatar: null, role: 'admin', connected: true, deviceCount: 2, lastSeenAt: new Date().toISOString(), createdAt: new Date(now - 90 * 86400000).toISOString(), discordLinked: true, discordUserId: '123456789012345678', isSelf: true },
    { id: 'm-lea', displayName: 'Léa', avatar: null, role: 'member', connected: true, deviceCount: 1, lastSeenAt: new Date().toISOString(), createdAt: new Date(now - 30 * 86400000).toISOString(), discordLinked: true, discordUserId: '234567890123456789', isSelf: false },
    { id: 'm-max', displayName: 'Max', avatar: null, role: 'member', connected: false, deviceCount: 1, lastSeenAt: new Date(now - 7200000).toISOString(), createdAt: new Date(now - 7 * 86400000).toISOString(), discordLinked: false, discordUserId: null, isSelf: false },
    { id: 'm-42', displayName: 'Invité-42', avatar: null, role: 'guest', connected: false, deviceCount: 1, lastSeenAt: null, createdAt: new Date(now - 86400000).toISOString(), discordLinked: false, discordUserId: null, isSelf: false },
  ];
  // Appareils de la guilde (`GET /overlay/admin/devices`) : quatre appareils
  // sur trois membres, un hors ligne (Max, en 3.5.2) et donc obsolète.
  const devices = [
    { id: 'd-111', label: 'Salon principal', memberId: 'm-111', memberName: 'Quentin', role: 'admin', sessionMode: 'normal', connected: true, lastSeenAt: new Date().toISOString(), overlayVersion: '3.7.0', platform: 'windows', guildSettings: { volume: 0.8, mediaMuted: false, paused: false }, isSelf: true },
    { id: 'd-lea', label: 'PC de Léa', memberId: 'm-lea', memberName: 'Léa', role: 'member', sessionMode: 'normal', connected: true, lastSeenAt: new Date(now - 5 * 60000).toISOString(), overlayVersion: '3.7.0', platform: 'windows', guildSettings: { volume: 1, mediaMuted: false, paused: false }, isSelf: false },
    { id: 'd-max', label: 'Salon', memberId: 'm-max', memberName: 'Max', role: 'member', sessionMode: 'normal', connected: false, lastSeenAt: new Date(now - 3 * 3600000).toISOString(), overlayVersion: '3.5.2', platform: 'linux', guildSettings: null, isSelf: false },
    { id: 'd-max-2', label: 'Portable de Max', memberId: 'm-max', memberName: 'Max', role: 'member', sessionMode: 'invite_read_only', connected: true, lastSeenAt: new Date(now - 2 * 60000).toISOString(), overlayVersion: '3.7.0', platform: 'macos', guildSettings: { volume: 0.6, mediaMuted: false, paused: true }, isSelf: false },
  ];
  let nextDeviceCode = 1;
  // Guild profile edited by « Mon profil »: the mock keeps it between dialogs.
  // It starts linked to the Discord account, as after a personal /pair-code.
  const discordProfile = { displayName: 'Quentin', avatar: null };
  // `?unlinked` starts with a member created without Discord (admin panel).
  const unlinked = new URLSearchParams(location.search).has('unlinked');
  const memberProfile = unlinked
    ? { displayName: 'Quentin', avatar: null, discordLinked: false, discordUserId: null, profileSource: 'custom' }
    : { ...discordProfile, discordLinked: true, discordUserId: '123456789012345678', profileSource: 'discord' };
  const invites = [
    { code: 'K7QXM2PD', role: 'member', maxUses: 1, useCount: 0, expiresAt: new Date(now + 22 * 3600000).toISOString(), revokedAt: null, isActive: true },
    { code: 'P3WN8RTA', role: 'guest', maxUses: 10, useCount: 4, expiresAt: new Date(now + 6 * 86400000).toISOString(), revokedAt: null, isActive: true },
    { code: 'Z9LCV5HE', role: 'admin', maxUses: 1, useCount: 1, expiresAt: null, revokedAt: null, isActive: false },
  ];
  // Codes d'appairage (`pairing_codes`), comme les sert
  // `GET /overlay/admin/pairing-codes` : un code Discord actif et un code
  // d'extension expiré. La purge et la révocation modifient cette liste.
  const pairingCodes = [
    { code: 'PAIR42', guildId: '111', createdByDiscordUserId: '1001', authorName: 'Discordeur', expiresAt: new Date(now + 10 * 60000).toISOString(), usedAt: null, createdAt: new Date(now - 60000).toISOString(), kind: 'discord' },
    { code: 'EXT789', guildId: '111', createdByDiscordUserId: 'member:m-lea', authorName: 'Léa', expiresAt: new Date(now - 60000).toISOString(), usedAt: null, createdAt: new Date(now - 3600000).toISOString(), kind: 'extension' },
  ];
  const pairingCodeActive = (entry) => !entry.usedAt && (!entry.expiresAt || new Date(entry.expiresAt).getTime() > Date.now());
  const pairingCodePayload = (entry) => ({ ...entry, isActive: pairingCodeActive(entry) });

  const permissions = {
    viewBoard: true,
    viewQueue: true,
    play: !isGuest,
    stop: !isGuest,
    addMedia: !isGuest,
    editAnyMedia: isAdmin,
    moderateQueue: isAdmin,
    manageMembers: isAdmin,
    manageInvites: isAdmin,
    manageSettings: isAdmin,
  };

  function svg(index) {
    if (realMedia && REAL_MEDIA[index]) {
      return new URL('./fixtures/' + REAL_MEDIA[index], location.href).href;
    }
    return placeholder(index);
  }

  // Generated image (never a fixture): used for covers and thumbnails, which
  // must stay images even when `?media` maps the media itself to a video.
  function placeholder(index) {
    const hue = (index * 47) % 360;
    const [mediaWidth, mediaHeight] = sizeOf(index);
    const width = 320;
    const height = Math.round((320 * mediaHeight) / mediaWidth);
    return 'data:image/svg+xml,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="${width}" height="${height}" fill="hsl(${hue} 35% 32%)"/><circle cx="${Math.min(60 + index * 20, width - 50)}" cy="${Math.round(height / 2)}" r="46" fill="hsl(${hue} 45% 52%)"/></svg>`);
  }

  // Device settings: the mock validates the same fields as the Rust shell and
  // always answers with a deep copy.
  const ALIGNMENTS = new Set([
    'center', 'top', 'bottom', 'left', 'right', 'top-left', 'top-right', 'bottom-left', 'bottom-right',
  ]);
  const MIN_ZONE_FRACTION = 0.05;
  const fullZone = () => ({ x: 0, y: 0, width: 1, height: 1 });
  // Same rules as `LayoutZone::normalize` in the shell.
  function normalizeZone(zone) {
    const round = (value) => Math.round(value * 10000) / 10000;
    const clamp = (value, min, max) => Math.min(max, Math.max(min, value));
    const width = round(clamp(zone.width, MIN_ZONE_FRACTION, 1));
    const height = round(clamp(zone.height, MIN_ZONE_FRACTION, 1));
    return { x: round(clamp(zone.x, 0, 1 - width)), y: round(clamp(zone.y, 0, 1 - height)), width, height };
  }
  const settingsCopy = () => JSON.parse(JSON.stringify(settings));
  const settingsError = (field, message) => ({ ok: false, field, message, settings: settingsCopy() });
  // Draft layouts sent by the live preview, oldest first.
  const previewLayouts = [];

  function validAccelerator(value) {
    if (typeof value !== 'string' || /bad/i.test(value)) {
      return false;
    }
    const parts = value.split('+');
    if (parts.length < 2) {
      return false;
    }
    const modifiers = /^(CommandOrControl|Command|Cmd|Control|Ctrl|Alt|Option|Shift|Super|Meta)$/i;
    if (!parts.slice(0, -1).every((part) => modifiers.test(part))) {
      return false;
    }
    return /^[A-Za-z0-9]+$/.test(parts[parts.length - 1]);
  }

  function applySettingsPatch(patch) {
    const input = patch && typeof patch === 'object' ? patch : {};
    const next = settingsCopy();
    if ('selectedDisplayKey' in input) {
      const key = input.selectedDisplayKey;
      if (key !== null && !next.displays.some((display) => display.key === key)) {
        return settingsError('selectedDisplayKey', 'Écran inconnu.');
      }
      next.selectedDisplayKey = key;
    }
    if ('volume' in input) {
      if (typeof input.volume !== 'number' || !Number.isFinite(input.volume)) {
        return settingsError('volume', 'Volume numérique attendu.');
      }
      next.volume = Math.min(1, Math.max(0, input.volume));
    }
    if ('guilds' in input) {
      const patches = input.guilds;
      if (!patches || typeof patches !== 'object') {
        return settingsError('guilds', 'Réglages de guilde invalides.');
      }
      for (const [guildId, own] of Object.entries(patches)) {
        // Unknown guilds are ignored, like the shell's membership lookup.
        const target = next.guilds[guildId];
        if (!target || !own || typeof own !== 'object') {
          continue;
        }
        if ('volume' in own) {
          if (typeof own.volume !== 'number' || !Number.isFinite(own.volume)) {
            return settingsError(`guilds.${guildId}.volume`, 'Volume numérique attendu.');
          }
          target.volume = Math.min(1, Math.max(0, own.volume));
        }
        if (typeof own.mediaMuted === 'boolean') {
          target.mediaMuted = own.mediaMuted;
        }
        if ('showText' in own) {
          target.showText = own.showText === null ? null : Boolean(own.showText);
        }
        if ('layout' in own) {
          target.layout = own.layout === null ? null : { ...(target.layout || next.layout), ...own.layout, zone: { ...(target.layout || next.layout).zone, ...(own.layout.zone || {}) } };
        }
      }
    }
    for (const key of ['generalMute', 'mediaMuted', 'showText', 'hoverDetails', 'autoStart']) {
      if (!(key in input)) {
        continue;
      }
      if (typeof input[key] !== 'boolean') {
        return settingsError(key, 'Valeur booléenne attendue.');
      }
      next[key] = input[key];
    }
    if ('shortcuts' in input) {
      const shortcuts = input.shortcuts;
      if (shortcuts === null) {
        next.shortcuts = { stop: null, stopLocal: null, muteAll: null };
      } else if (!shortcuts || typeof shortcuts !== 'object') {
        return settingsError('shortcuts', 'Raccourcis invalides.');
      } else {
        for (const [action, value] of Object.entries(shortcuts)) {
          if (!(action in next.shortcuts)) {
            return settingsError(`shortcuts.${action}`, 'Raccourci inconnu.');
          }
          if (value !== null && !validAccelerator(value)) {
            return settingsError(`shortcuts.${action}`, 'Raccourci invalide.');
          }
          next.shortcuts[action] = value;
        }
        // Like the shell: one accelerator cannot drive two actions.
        const taken = new Map();
        for (const [action, value] of Object.entries(next.shortcuts)) {
          if (!value) {
            continue;
          }
          if (taken.has(value)) {
            const field = action in shortcuts ? action : taken.get(value);
            return settingsError(`shortcuts.${field}`, 'Ce raccourci sert déjà à une autre action.');
          }
          taken.set(value, action);
        }
      }
    }
    if ('layout' in input) {
      const layout = input.layout;
      if (layout === null) {
        next.layout = { zone: fullZone(), align: 'center' };
      } else if (!layout || typeof layout !== 'object') {
        return settingsError('layout', 'objet JSON attendu');
      } else {
        // Like the shell, other keys are ignored.
        if ('align' in layout) {
          if (layout.align === null) {
            next.layout.align = 'center';
          } else if (ALIGNMENTS.has(layout.align)) {
            next.layout.align = layout.align;
          } else {
            return settingsError('layout.align', 'alignement inconnu');
          }
        }
        if ('zone' in layout) {
          const zone = layout.zone;
          if (zone === null) {
            next.layout.zone = fullZone();
          } else if (!zone || typeof zone !== 'object' || Array.isArray(zone)) {
            return settingsError('layout.zone', 'objet JSON ou null attendu');
          } else {
            const merged = { ...next.layout.zone };
            for (const key of ['x', 'y', 'width', 'height']) {
              if (!(key in zone)) {
                continue;
              }
              if (typeof zone[key] !== 'number' || !Number.isFinite(zone[key])) {
                return settingsError(`layout.zone.${key}`, 'nombre attendu');
              }
              merged[key] = zone[key];
            }
            next.layout.zone = normalizeZone(merged);
          }
        }
      }
    }
    if ('profile' in input) {
      const profile = input.profile;
      if (profile === null) {
        next.profile = { displayName: null, avatar: null };
      } else if (!profile || typeof profile !== 'object') {
        return settingsError('profile', 'Profil invalide.');
      } else {
        for (const [key, value] of Object.entries(profile)) {
          if (key !== 'displayName' && key !== 'avatar') {
            return settingsError(`profile.${key}`, 'Champ de profil inconnu.');
          }
          if (value !== null && typeof value !== 'string') {
            return settingsError(`profile.${key}`, 'Valeur invalide.');
          }
          next.profile[key] = value;
        }
      }
    }
    Object.assign(settings, next);
    emit('overlay://device-settings', settingsCopy());
    return { ok: true, settings: settingsCopy() };
  }

  // Mock updater: a check finds 3.2.0, downloads it, then it waits to be installed.
  function setUpdate(state, availableVersion, message) {
    settings.update = { state, currentVersion: settings.version, availableVersion, message };
    emit('overlay://device-settings', settingsCopy());
    return { ...settings.update };
  }

  function checkForUpdate() {
    if (settings.update.state === 'ready') {
      return Promise.resolve({ ...settings.update });
    }
    setUpdate('checking', null, 'Recherche d’une mise à jour…');
    return delay(300)
      .then(() => setUpdate('downloading', '3.2.0', 'Téléchargement de la version 3.2.0…'))
      .then(() => delay(600))
      .then(() => setUpdate('ready', '3.2.0', 'La version 3.2.0 est prête à être installée.'));
  }

  function api(payload) {
    const { method, path } = payload;
    const ok = (body) => ({ ok: true, status: 200, body });
    if (path === '/overlay/media-board/items' && method === 'POST') {
      const body = payload.body || {};
      const duration = playOnceDisplayDuration(body);
      if (!duration.ok) {
        return { ok: false, status: 400, body: { error: 'invalid_payload', message: duration.message } };
      }
      return acceptIngestion({ guildId: payload.guildId, name: String(body.url || ''), title: body.title, message: body.playOnce ? body.message : null, contentType: 'video/mp4', received: 0, playOnce: Boolean(body.playOnce), displayDurationSec: duration.value });
    }
    if (path === '/overlay/say' && method === 'POST') {
      // Speech synthesis (/dire). A text containing "panne" fails like a
      // broken Piper, to preview the error state.
      const body = payload.body || {};
      const spoken = String(body.text || '').split(/\s+/).filter(Boolean).join(' ');
      if (!spoken) {
        return { ok: false, status: 400, body: { error: 'invalid_payload' } };
      }
      if (Array.from(spoken).length > 1000) {
        return { ok: false, status: 400, body: { error: 'text_too_long', maxChars: 1000 } };
      }
      if (spoken.includes('panne')) {
        return delay(600).then(() => ({ ok: false, status: 502, body: { error: 'speech_failed' } }));
      }
      const job = { id: 'j' + nextJob++, status: 'pending', title: null, text: body.caption || null, authorName: body.anonymous ? null : 'Quentin', durationSec: Math.max(1, Math.round(spoken.length / 15)), kind: 'audio', origin: 'direct' };
      return delay(600).then(() => {
        queue.pending.push(job);
        queueHint();
        return ok({ queued: true, jobId: job.id, durationSec: job.durationSec });
      });
    }
    if (path === '/overlay/media-board/ingestions' && method === 'GET') {
      return ok({ items: ingestionList(payload.guildId) });
    }
    const editPath = /^\/overlay\/media-board\/items\/([^/?]+)\/edit$/.exec(path);
    if (editPath && method === 'POST') {
      const source = items.find((candidate) => candidate.id === decodeURIComponent(editPath[1]));
      const body = payload.body || {};
      requests.push({ method, path, body });
      if (!source) {
        return { ok: false, status: 404, body: { error: 'item_not_found' } };
      }
      if (!body.recipe || typeof body.recipe !== 'object') {
        return { ok: false, status: 400, body: { error: 'invalid_payload', message: 'recipe is required' } };
      }
      const output = body.recipe.output || 'same';
      const kind = output === 'audio' ? 'audio' : output === 'gif' ? 'image' : source.media.kind;
      const kept = (body.recipe.segments || [[0, source.media.durationSec || 0]]).reduce((total, range) => total + (range[1] - range[0]), 0);
      const durationSec = kind === 'image' ? null : Math.max(1, Math.ceil(kept / (body.recipe.speed || 1)));
      return acceptIngestion({ guildId: payload.guildId, name: String(body.title || ''), title: body.title, message: source.message, contentType: kind + '/x', received: source.media.sizeBytes, replaceItemId: body.replace ? source.id : null, mediaUrl: source.mediaUrl, durationSec });
    }
    const favoritePath = /^\/overlay\/media-board\/items\/([^/?]+)\/(favorite|artwork)$/.exec(path);
    if (favoritePath) {
      const entry = items.find((candidate) => candidate.id === decodeURIComponent(favoritePath[1]));
      if (!entry) {
        return { ok: false, status: 404, body: { error: 'item_not_found' } };
      }
      if (favoritePath[2] === 'favorite' && method === 'PUT') {
        entry.favorite = Boolean(payload.body && payload.body.favorite);
        return ok({ itemId: entry.id, favorite: entry.favorite });
      }
      if (favoritePath[2] === 'artwork' && method === 'DELETE') {
        entry.artwork = null;
        boardChanged(payload.guildId, 'updated', entry.id);
        return ok({ itemId: entry.id });
      }
    }
    const itemPath = /^\/overlay\/media-board\/items\/([^/?]+)$/.exec(path);
    if (itemPath && (method === 'PATCH' || method === 'DELETE')) {
      const itemId = decodeURIComponent(itemPath[1]);
      const index = items.findIndex((entry) => entry.id === itemId);
      if (index === -1) {
        return { ok: false, status: 404, body: { error: 'not_found' } };
      }
      if (method === 'DELETE') {
        items.splice(index, 1);
      } else {
        // Like the server: an absent field is kept, an empty one is cleared.
        const patch = payload.body || {};
        for (const field of ['title', 'message']) {
          if (field in patch) {
            items[index][field] = (typeof patch[field] === 'string' && patch[field].trim()) || null;
          }
        }
        if (payload.body && typeof payload.body.artworkOnOverlay === 'boolean') {
          items[index].artworkOnOverlay = payload.body.artworkOnOverlay;
        }
        if (payload.body && 'displayDurationSec' in payload.body) {
          const seconds = payload.body.displayDurationSec;
          const valid = seconds === null || (Number.isInteger(seconds) && seconds >= 1 && seconds <= 600);
          if (!valid || (seconds !== null && items[index].media.kind !== 'image')) {
            return { ok: false, status: 400, body: { error: 'invalid_payload' } };
          }
          items[index].displayDurationSec = seconds;
        }
        if (payload.body && 'displayScalePercent' in payload.body) {
          const percent = payload.body.displayScalePercent;
          const valid = percent === null || (Number.isInteger(percent) && percent >= 10 && percent <= 1000);
          if (!valid || (percent !== null && items[index].media.kind !== 'image')) {
            return { ok: false, status: 400, body: { error: 'invalid_payload' } };
          }
          items[index].displayScalePercent = percent;
        }
      }
      boardChanged(payload.guildId, method === 'DELETE' ? 'deleted' : 'updated', itemId);
      return ok(method === 'DELETE' ? { deleted: true } : { updated: true, item: items[index] });
    }
    if (path === '/overlay/profile/discord-link' && method === 'POST') {
      const code = String((payload.body && payload.body.code) || '').replace(/-/g, '').toUpperCase();
      if (code !== 'K7QXM2PD') {
        return { ok: false, status: 404, body: { error: 'pairing_code_invalid_or_expired' } };
      }
      memberProfile.discordLinked = true;
      return ok({ linked: true, promoted: false, member: memberProfile });
    }
    if (path === '/overlay/profile' && method === 'PATCH') {
      const body = payload.body || {};
      if (body.useDiscord) {
        if (!memberProfile.discordLinked) {
          return { ok: false, status: 409, body: { error: 'discord_not_linked' } };
        }
        Object.assign(memberProfile, discordProfile, { profileSource: 'discord' });
      }
      // Missing key keeps the value; an explicit null removes it.
      if (!body.useDiscord && 'displayName' in body) {
        memberProfile.displayName = body.displayName === null ? null : String(body.displayName);
      }
      if (!body.useDiscord && 'avatar' in body) {
        memberProfile.avatar = body.avatar === null ? null : String(body.avatar);
      }
      if (!body.useDiscord) {
        memberProfile.profileSource = 'custom';
      }
      const self = members.find((member) => member.id === 'm-111');
      if (self) {
        Object.assign(self, { displayName: memberProfile.displayName, avatar: memberProfile.avatar });
      }
      const member = { id: 'm-111', ...memberProfile, role };
      return ok(body.useDiscord ? { updated: true, synced: true, member } : { updated: true, member });
    }
    if (path === '/overlay/admin/settings' && method === 'PATCH') {
      const body = payload.body || {};
      if (!body || typeof body !== 'object') {
        return { ok: false, status: 400, body: { error: 'invalid_payload' } };
      }
      if ('name' in body) {
        const next = String(body.name || '').trim();
        if (!next) {
          return { ok: false, status: 400, body: { error: 'invalid_name' } };
        }
        guildName = next;
        const active = guilds.guilds.find((guild) => guild.membership.guildId === '111');
        if (active) {
          active.membership.guildName = next;
        }
      }
      if ('defaultMediaTime' in body) {
        guildSettings.defaultMediaTime = body.defaultMediaTime === null ? null : Math.max(0, Number(body.defaultMediaTime) || 0);
      }
      if ('maxMediaTime' in body) {
        guildSettings.maxMediaTime = body.maxMediaTime === null ? null : Math.max(0, Number(body.maxMediaTime) || 0);
      }
      if ('displayMediaFull' in body) {
        guildSettings.displayMediaFull = Boolean(body.displayMediaFull);
      }
      if ('guestsCanViewBoard' in body) {
        guestsCanViewBoard = Boolean(body.guestsCanViewBoard);
      }
      if ('boardQuotaMb' in body) {
        const quota = body.boardQuotaMb;
        if (quota === null || quota === '') {
          boardQuotaMb = null;
        } else if (typeof quota === 'number' && Number.isInteger(quota) && quota > 0) {
          if (quota > SERVER_BOARD_MAX_BYTES / (1024 * 1024)) {
            return { ok: false, status: 400, body: { error: 'invalid_board_quota' } };
          }
          boardQuotaMb = quota;
        } else {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
      }
      return ok({
        updated: true,
        guild: {
          id: '111',
          name: guildName,
          guestsCanViewBoard,
          settings: { ...guildSettings },
          boardQuotaMb,
        },
      });
    }
    if (path === '/overlay/admin/media-board/clear' && method === 'POST') {
      const body = payload.body || {};
      if (body.confirm !== guildName) {
        return { ok: false, status: 400, body: { error: 'confirmation_mismatch' } };
      }
      let releasedPlayingJobs = 0;
      if (queue.playing) {
        releasedPlayingJobs = 1;
        queueHistory.unshift({
          id: queue.playing.id,
          status: 'done',
          title: queue.playing.title,
          text: queue.playing.text,
          authorName: queue.playing.authorName,
          durationSec: queue.playing.durationSec,
          kind: queue.playing.kind,
          origin: queue.playing.origin || 'board',
          finishedAt: new Date().toISOString(),
        });
        queue.playing = null;
      }
      queue.pending = [];
      const removed = items.splice(0, items.length).length;
      boardChanged('111', 'deleted', null);
      return ok({
        guildId: '111',
        stop: { releasedPlayingJobs, broadcast: 0 },
        deleted: { mediaBoardItems: removed, orphanMediaAssets: removed, orphanMediaFileDeleteErrors: 0 },
      });
    }
    if (path.startsWith('/overlay/admin/audit') && method === 'GET') {
      const rawLimit = new URLSearchParams(path.split('?')[1] || '').get('limit');
      const parsed = rawLimit === null ? 50 : Number(rawLimit);
      const limit = Number.isFinite(parsed) ? Math.min(200, Math.max(1, Math.trunc(parsed))) : 50;
      const entries = auditEntries.slice(0, limit);
      return ok({ count: entries.length, items: entries });
    }
    if (path.startsWith('/overlay/queue/history') && method === 'GET') {
      const rawLimit = new URLSearchParams(path.split('?')[1] || '').get('limit');
      const parsed = rawLimit === null ? 30 : Number(rawLimit);
      const limit = Number.isFinite(parsed) ? Math.min(100, Math.max(1, Math.trunc(parsed))) : 30;
      const entries = queueHistory.slice(0, limit);
      return ok({ count: entries.length, items: entries });
    }
    if (path === '/overlay/guild') {
      const effectiveDefault = guildSettings.defaultMediaTime ?? 5;
      return ok({
        guild: {
          id: '111',
          name: guildName,
          standalone: true,
          discordGuildId: null,
          guestsCanViewBoard,
          settings: {
            defaultMediaTime: guildSettings.defaultMediaTime,
            maxMediaTime: guildSettings.maxMediaTime,
            displayMediaFull: guildSettings.displayMediaFull,
            effective: { defaultMediaTime: effectiveDefault, maxMediaTime: guildSettings.maxMediaTime },
          },
          mediaBoard: mediaBoardPayload(),
          playback: { done: 12, failed: 1, pending: queue.pending.length, busyUntil: null },
          connectedCount: 6,
        },
        member: { id: 'm-111', ...memberProfile, role },
        permissions,
      });
    }
    if (path.startsWith('/overlay/media-board/items') && method === 'GET') {
      // Copies: the board must not see the mock's arrays change under it.
      return ok({ total: items.length, items: items.slice() });
    }
    if (path === '/overlay/queue' && method === 'GET') {
      return ok(JSON.parse(JSON.stringify(queue)));
    }
    if (path === '/overlay/admin/members') {
      return ok({ count: members.length, items: members });
    }
    if (path === '/overlay/admin/devices' && method === 'GET') {
      return ok(JSON.parse(JSON.stringify({ count: devices.length, items: devices })));
    }
    const devicePath = /^\/overlay\/admin\/devices\/([^/?]+)(\/guild-settings)?$/.exec(path);
    if (devicePath) {
      const entry = devices.find((candidate) => candidate.id === decodeURIComponent(devicePath[1]));
      if (!entry) {
        return { ok: false, status: 404, body: { error: 'device_not_found' } };
      }
      if (devicePath[2] === '/guild-settings' && method === 'PATCH') {
        const body = payload.body || {};
        const fields = ['volume', 'mediaMuted', 'paused'];
        if (!fields.some((field) => field in body)) {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
        if ('volume' in body && (typeof body.volume !== 'number' || !Number.isFinite(body.volume) || body.volume < 0 || body.volume > 1)) {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
        if ('mediaMuted' in body && typeof body.mediaMuted !== 'boolean') {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
        if ('paused' in body && typeof body.paused !== 'boolean') {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
        entry.guildSettings = {
          volume: 'volume' in body ? body.volume : (entry.guildSettings || {}).volume ?? 1,
          mediaMuted: 'mediaMuted' in body ? body.mediaMuted : Boolean((entry.guildSettings || {}).mediaMuted),
          paused: 'paused' in body ? body.paused : Boolean((entry.guildSettings || {}).paused),
        };
        return ok({ delivered: entry.connected });
      }
      if (devicePath[2]) {
        return { ok: false, status: 404, body: { error: 'request_error' } };
      }
      if (method === 'DELETE') {
        if (entry.isSelf) {
          return { ok: false, status: 409, body: { error: 'cannot_revoke_self' } };
        }
        devices.splice(devices.indexOf(entry), 1);
        return ok({ found: true, revoked: true });
      }
      if (method === 'PATCH') {
        const body = payload.body || {};
        const hasLabel = 'label' in body;
        const hasMode = 'sessionMode' in body;
        if (!hasLabel && !hasMode) {
          return { ok: false, status: 400, body: { error: 'invalid_payload' } };
        }
        if (hasLabel) {
          if (typeof body.label !== 'string') {
            return { ok: false, status: 400, body: { error: 'invalid_payload' } };
          }
          const label = body.label.trim();
          if (!label || Array.from(label).length > 40) {
            return { ok: false, status: 400, body: { error: 'invalid_payload' } };
          }
          entry.label = label;
        }
        if (hasMode) {
          if (body.sessionMode !== 'normal' && body.sessionMode !== 'invite_read_only') {
            return { ok: false, status: 400, body: { error: 'invalid_payload' } };
          }
          entry.sessionMode = body.sessionMode;
        }
        return ok(entry);
      }
    }
    if (path === '/overlay/admin/device-codes' && method === 'POST') {
      const memberId = payload.body && payload.body.memberId;
      const member = members.find((candidate) => candidate.id === memberId);
      if (!member) {
        return { ok: false, status: 404, body: { error: 'member_not_found' } };
      }
      const suffix = String(nextDeviceCode++).padStart(5, '0');
      const at = new Date().toISOString();
      return ok({
        created: true,
        invite: { code: `D3V${suffix}`, role: member.role, memberId: member.id, maxUses: 1, useCount: 0, createdByMemberId: 'm-111', expiresAt: new Date(Date.now() + 10 * 60000).toISOString(), revokedAt: null, createdAt: at, isActive: true },
      });
    }
    if (path === '/overlay/admin/invites' && method === 'GET') {
      return ok({ count: invites.length, items: invites });
    }
    if (path === '/overlay/device-code') {
      return ok({ created: true, invite: { code: 'D3V1C3AB' } });
    }
    if (path === '/overlay/admin/pairing-codes' && method === 'GET') {
      const items = pairingCodes.map(pairingCodePayload);
      return ok({ count: items.length, items });
    }
    const pairingCodeRevoke = /^\/overlay\/admin\/pairing-codes\/([^/?]+)\/revoke$/.exec(path);
    if (pairingCodeRevoke && method === 'POST') {
      const code = decodeURIComponent(pairingCodeRevoke[1]);
      const entry = pairingCodes.find((candidate) => candidate.code === code);
      if (!entry) {
        return { ok: false, status: 404, body: { error: 'pairing_code_not_found' } };
      }
      const revoked = pairingCodeActive(entry);
      if (revoked) {
        const at = new Date().toISOString();
        entry.usedAt = at;
        entry.expiresAt = at;
      }
      return ok({ revoked });
    }
    if (path === '/overlay/admin/pairing-codes/expired' && method === 'DELETE') {
      let deleted = 0;
      for (let index = pairingCodes.length - 1; index >= 0; index -= 1) {
        if (!pairingCodeActive(pairingCodes[index])) {
          pairingCodes.splice(index, 1);
          deleted += 1;
        }
      }
      return ok({ deleted });
    }
    return ok({ created: true, updated: true, removed: true, deleted: true, invite: invites[0] });
  }

  // Events: `listen` registers a callback id, `__mock.emit` calls it.
  const callbacks = new Map();
  const listeners = new Map();
  let nextCallback = 1;
  function emit(event, payload) {
    for (const id of listeners.get(event) || []) {
      const callback = callbacks.get(id);
      if (callback) {
        callback({ event, payload });
      }
    }
  }
  const queueHint = () => setTimeout(() => emit('overlay://queue-hint', { guildId: '111' }), 150);

  let nextJob = 10;
  function trigger(itemId) {
    const item = items.find((entry) => entry.id === itemId);
    if (!item) {
      return { ok: false, reason: 'item_not_found' };
    }
    if (item.title === 'Sad violin') {
      return { ok: false, reason: 'media_not_ready' };
    }
    const job = { id: 'j' + nextJob++, title: item.title, text: item.message, authorName: 'Quentin', durationSec: item.media.durationSec || 8, kind: item.media.kind, origin: 'board' };
    queueHint();
    if (!queue.playing) {
      const at = new Date().toISOString();
      queue.playing = Object.assign(job, { status: 'playing', startedAt: at, submittedAt: at });
      return { ok: true, outcome: 'playing', jobId: job.id };
    }
    queue.pending.push(Object.assign(job, { status: 'pending' }));
    return { ok: true, outcome: 'queued', position: queue.pending.length, jobId: job.id };
  }
  // Job stopped on this device only (`media_board_stop_local`).
  let locallyStoppedJobId = null;
  function finishPlaying() {
    if (queue.playing) {
      queueHistory.unshift({
        id: queue.playing.id,
        status: 'done',
        title: queue.playing.title,
        text: queue.playing.text,
        authorName: queue.playing.authorName,
        durationSec: queue.playing.durationSec,
        kind: queue.playing.kind,
        origin: queue.playing.origin || 'board',
        finishedAt: new Date().toISOString(),
      });
    }
    queue.playing = queue.pending.shift() || null;
    if (queue.playing) {
      Object.assign(queue.playing, { status: 'playing', startedAt: new Date().toISOString() });
    }
    queueHint();
  }

  // Local file uploads: chunks are counted, finish answers like the server.
  const uploads = new Map();
  let nextUpload = 1;
  const delay = (ms, value) => new Promise((resolve) => setTimeout(() => resolve(value), ms));

  // Background ingestions, as the server keeps them: 202 at once, then the
  // result over the WebSocket of the requesting device only.
  const ingestions = new Map();
  let nextIngestion = 1;
  let nextItem = items.length;
  function boardChanged(guildId, change, itemId) {
    setTimeout(() => emit('overlay://board-changed', { guildId: guildId || '111', change, itemId }), 50);
  }
  function ingestionList(guildId) {
    return Array.from(ingestions.values())
      .filter((entry) => entry.guildId === (guildId || '111'))
      .map(({ ingestionId, status, created, itemId, jobId, error }) => ({ ingestionId, status, created, itemId, jobId, error }));
  }
  function createItem(source) {
    const index = nextItem++;
    const kind = String(source.contentType || '').split('/')[0];
    const item = {
      id: 'item-' + index,
      guildId: source.guildId || '111',
      mediaAssetId: 'a' + index,
      mediaUrl: svg(index),
      mediaUrlExpiresAt: Date.now() + 3600000,
      title: source.title || null,
      message: source.message || null,
      createdByName: 'Quentin',
      createdByMemberId: 'm-111',
      canEdit: true,
      createdAt: new Date().toISOString(),
      media: { id: 'a' + index, kind: ['video', 'image', 'audio'].includes(kind) ? kind : 'video', mime: source.contentType || 'x', durationSec: kind === 'image' ? null : 6, width: 1280, height: 720, isVertical: false, sizeBytes: source.received, sourceUrl: null },
      displayDurationSec: null,
      displayScalePercent: null,
    };
    items.unshift(item);
    return item;
  }
  // The result reaches the board only while the guild's WebSocket is up;
  // otherwise the board must find it with GET /overlay/media-board/ingestions.
  function completeIngestion(ingestion, source) {
    if (!ingestions.has(ingestion.ingestionId)) {
      return; // Lost by a simulated server restart.
    }
    const name = source.name.toLowerCase();
    if (name.includes('fail')) {
      Object.assign(ingestion, { status: 'failed', error: { code: 'media_ingestion_failed', message: 'la vidéo dépasse la durée maximale de 120 s' } });
    } else if (source.playOnce) {
      // Like the server: the ready media becomes a playback job, no board
      // item and no `board.changed`. The chosen display time only applies to
      // an image; a video or an audio keeps its own length.
      const kind = String(source.contentType || '').split('/')[0];
      const durationSec = kind === 'image' ? source.displayDurationSec || 8 : 6;
      const job = { id: 'j' + nextJob++, title: source.title || null, text: source.message || null, authorName: 'Quentin', durationSec, kind: ['video', 'image', 'audio'].includes(kind) ? kind : 'video', status: 'pending', origin: 'direct' };
      queueHint();
      if (!queue.playing) {
        const at = new Date().toISOString();
        queue.playing = Object.assign(job, { status: 'playing', startedAt: at, submittedAt: at });
      } else {
        queue.pending.push(job);
      }
      Object.assign(ingestion, { status: 'ready', created: false, jobId: job.id });
    } else if (name.includes('dup')) {
      Object.assign(ingestion, { status: 'ready', created: false, itemId: items[0] ? items[0].id : 'item-0' });
    } else if (source.replaceItemId) {
      // An edit replacing its original: the item keeps its id, the media changes.
      const item = items.find((candidate) => candidate.id === source.replaceItemId);
      if (item) {
        const assetId = 'a' + nextItem++;
        item.mediaAssetId = assetId;
        item.media = { ...item.media, id: assetId, durationSec: source.durationSec === undefined ? item.media.durationSec : source.durationSec, sizeBytes: source.received || item.media.sizeBytes };
        Object.assign(ingestion, { status: 'ready', created: false, itemId: item.id });
        boardChanged(ingestion.guildId, 'updated', item.id);
      } else {
        Object.assign(ingestion, { status: 'failed', error: { code: 'item_not_found' } });
      }
    } else {
      const item = createItem(source);
      if (source.mediaUrl) {
        item.mediaUrl = source.mediaUrl;
      }
      if (source.durationSec !== undefined) {
        item.media.durationSec = source.durationSec;
      }
      Object.assign(ingestion, { status: 'ready', created: true, itemId: item.id });
      boardChanged(ingestion.guildId, 'added', item.id);
    }
    const guild = guilds.guilds.find((entry) => entry.membership.guildId === ingestion.guildId);
    if (guild && guild.connection.status === 'connected') {
      const { ingestionId, status, created, itemId, jobId, error } = ingestion;
      emit('overlay://media-ingestion', { guildId: ingestion.guildId, ingestionId, ok: status === 'ready', created, itemId, jobId, error });
    }
  }
  function acceptIngestion(source) {
    const name = source.name.toLowerCase();
    const ingestion = { ingestionId: 'ing-' + nextIngestion++, guildId: source.guildId || '111', status: 'processing' };
    ingestions.set(ingestion.ingestionId, ingestion);
    const accepted = { ok: true, status: 202, body: { ingestionId: ingestion.ingestionId } };
    if (name.includes('race')) {
      setTimeout(() => completeIngestion(ingestion, source), 0);
      return delay(400, accepted);
    }
    setTimeout(() => completeIngestion(ingestion, source), name.includes('slow') ? 20000 : 2500);
    return accepted;
  }
  // `displayDurationSec` only belongs to a play once. A link's kind is unknown
  // before the download, so the mock checks the bounds only, like the server;
  // the kind filter happens when the ingestion completes.
  function playOnceDisplayDuration(body) {
    const raw = body && body.displayDurationSec;
    if (raw === undefined || raw === null) {
      return { ok: true, value: null };
    }
    if (!body.playOnce) {
      return { ok: false, message: 'displayDurationSec is only allowed for a play once' };
    }
    if (!Number.isInteger(raw) || raw < 1 || raw > 600) {
      return { ok: false, message: 'displayDurationSec must be an integer between 1 and 600, or null' };
    }
    return { ok: true, value: raw };
  }
  function finishUpload(upload) {
    const name = String(upload.filename || '');
    const duration = playOnceDisplayDuration(upload);
    if (!duration.ok) {
      return { ok: false, status: 400, body: { error: 'invalid_payload', message: duration.message } };
    }
    if (name.toLowerCase().includes('big')) {
      return { ok: false, status: 413, body: { error: 'payload_too_large', maxBytes: 2048 * 1024 * 1024 } };
    }
    return acceptIngestion({ guildId: upload.guildId, name, title: upload.title, message: upload.playOnce ? upload.message : null, contentType: upload.contentType, received: upload.received, replaceItemId: upload.replaceItemId || null, playOnce: Boolean(upload.playOnce), displayDurationSec: duration.value });
  }
  // The shell sends a fresh copy with every event, never the object the board
  // already holds: the board compares the old and new connection states.
  const guildsCopy = () => JSON.parse(JSON.stringify(guilds));
  function setConnection(guildId, status, label) {
    const guild = guilds.guilds.find((entry) => entry.membership.guildId === guildId);
    if (guild) {
      guild.connection.status = status;
      guild.statusLabel = label;
      emit('overlay://guilds', guildsCopy());
    }
  }
  function reconnect(guildId) {
    const id = guildId || '111';
    setConnection(id, 'reconnecting', 'reconnexion…');
    setTimeout(() => setConnection(id, 'connected', 'connecté'), 3000);
  }
  function restartServer() {
    for (const [id, entry] of ingestions) {
      if (entry.status === 'processing') {
        ingestions.delete(id);
      }
    }
    reconnect('111');
  }

  const removals = new URLSearchParams(location.search).has('removed')
    ? [{ guildId: '999', guildName: 'Ancienne guilde', reason: 'guild_deleted' }]
    : [];
  function removeGuild(guildId, reason) {
    const index = guilds.guilds.findIndex((guild) => guild.membership.guildId === guildId);
    const name = index >= 0 ? guilds.guilds[index].membership.guildName : null;
    if (index >= 0) {
      guilds.guilds.splice(index, 1);
    }
    if (guilds.activeGuildId === guildId) {
      guilds.activeGuildId = guilds.guilds.length ? guilds.guilds[0].membership.guildId : null;
      guilds.guilds.forEach((guild) => {
        guild.membership.active = guild.membership.guildId === guilds.activeGuildId;
      });
    }
    removals.push({ guildId, guildName: name, reason: reason || 'guild_deleted' });
    emit('overlay://guild-removed', removals[removals.length - 1]);
    emit('overlay://guilds', guildsCopy());
  }

  // What the overlay does on « Valider » (a zone) or « Annuler » (null).
  let placementGuildId = null;

  function finishPlacement(zone) {
    if (!settings.placing) {
      return null;
    }
    settings.placing = false;
    const guild = placementGuildId ? settings.guilds[placementGuildId] : null;
    placementGuildId = null;
    if (zone && guild) {
      guild.layout = { ...(guild.layout || settings.layout), zone: normalizeZone(zone) };
    } else if (zone) {
      settings.layout.zone = normalizeZone(zone);
    }
    emit('overlay://device-settings', settingsCopy());
    return null;
  }

  function openSettingsPage() {
    emit('overlay://navigate', { view: 'settings' });
  }

  // Mute closes the guild's socket in the shell: the overlay goes offline for
  // the server, and comes back through connecting → connected.
  function applyMuted(guildId, muted) {
    const self = members.find((member) => member.isSelf);
    if (muted) {
      setTimeout(() => {
        if (guildId === '111' && self) {
          self.connected = false;
          self.lastSeenAt = new Date().toISOString();
        }
        setConnection(guildId, 'disabled', 'désactivé');
      }, 50);
      return;
    }
    setTimeout(() => setConnection(guildId, 'connecting', 'connexion…'), 50);
    setTimeout(() => {
      if (guildId === '111' && self) {
        self.connected = true;
      }
      setConnection(guildId, 'connected', 'connecté');
    }, 400);
  }

  const requests = [];

  window.__mock = { emit, removeGuild, finishPlaying, reconnect, restartServer, openSettings: openSettingsPage, finishPlacement, settings, previewLayouts, queue, items, guilds, ingestions, invites, pairingCodes, devices, requests };

  window.__TAURI_INTERNALS__ = {
    transformCallback(callback) {
      const id = nextCallback++;
      callbacks.set(id, callback);
      return id;
    },
    invoke(command, args, options) {
      const payload = (args && args.payload) || {};
      switch (command) {
        case 'plugin:event|listen':
          listeners.set(args.event, (listeners.get(args.event) || []).concat(args.handler));
          return Promise.resolve(args.handler);
        case 'media_upload_start': {
          const uploadId = 'up-' + nextUpload++;
          uploads.set(uploadId, Object.assign({ received: 0, chunks: 0 }, payload));
          return delay(100, { uploadId });
        }
        case 'media_upload_chunk': {
          const upload = uploads.get(options && options.headers && options.headers['x-upload-id']);
          if (!upload) {
            return Promise.reject(new Error('upload_not_found'));
          }
          upload.received += (args && args.byteLength) || 0;
          upload.chunks += 1;
          return delay(400, null);
        }
        case 'media_upload_finish': {
          const upload = uploads.get(payload.uploadId);
          uploads.delete(payload.uploadId);
          return upload ? delay(300).then(() => finishUpload(upload)) : Promise.reject(new Error('upload_not_found'));
        }
        case 'media_upload_cancel':
          return Promise.resolve(uploads.delete(payload.uploadId));
        case 'media_artwork_upload': {
          const headers = (options && options.headers) || {};
          const entry = items.find((candidate) => candidate.id === headers['x-item-id']);
          requests.push({ method: 'PUT', path: `/overlay/media-board/items/${headers['x-item-id']}/artwork`, filename: headers['x-filename'], bytes: (args && args.byteLength) || 0 });
          if (!entry) {
            return delay(100, { ok: false, status: 404, body: { error: 'item_not_found' } });
          }
          entry.artwork = { url: svg(1), width: 640, height: 360 };
          boardChanged(headers['x-guild-id'], 'updated', entry.id);
          return delay(200, { ok: true, status: 200, body: { itemId: entry.id } });
        }
        case 'board_take_guild_removals':
          return Promise.resolve(removals.splice(0));
        case 'board_take_pending_view': {
          const view = pendingView;
          pendingView = null;
          return Promise.resolve(view);
        }
        case 'settings_get':
          return delay(60).then(() => settingsCopy());
        case 'settings_set':
          return delay(60).then(() => applySettingsPatch(payload));
        case 'settings_identify_displays':
          return delay(200).then(() => ({ count: settings.displays.length }));
        case 'settings_test_media':
          return delay(120).then(() => (queue.playing ? { ok: false, reason: 'overlay_busy' } : { ok: true }));
        case 'settings_preview_layout':
          previewLayouts.push(JSON.parse(JSON.stringify(payload.guildId ? { ...payload.layout, guildId: payload.guildId } : payload.layout)));
          return Promise.resolve(null);
        case 'settings_placement_start':
          return delay(120).then(() => {
            settings.placing = true;
            placementGuildId = (payload && payload.guildId) || null;
            emit('overlay://device-settings', settingsCopy());
            return { ok: true };
          });
        case 'settings_placement_cancel':
          return delay(60).then(() => finishPlacement(null));
        case 'overlay_check_updates':
          return checkForUpdate();
        case 'overlay_install_update':
          return delay(200).then(() => Promise.reject('Mock : l’installeur ne peut pas être lancé dans le navigateur.'));
        case 'guilds_list':
          return Promise.resolve(guildsCopy());
        case 'guild_api':
          requests.push({ method: payload.method, path: payload.path });
          return delay(80).then(() => api(payload));
        case 'board_import':
          // Like the server: the first media is already on the target board,
          // the others are copied.
          requests.push({ method: 'POST', path: '/overlay/media-board/import', payload });
          return delay(150).then(() => ({
            ok: true,
            status: 200,
            body: {
              results: (payload.itemIds || []).map((id, index) =>
                index === 0 && payload.itemIds.length > 1
                  ? { sourceItemId: id, status: 'already', itemId: `copy-${id}` }
                  : { sourceItemId: id, status: 'added', itemId: `copy-${id}` }),
            },
          }));
        case 'media_board_get_bindings':
          return Promise.resolve({ bindings: { 'CommandOrControl+Shift+1': 'item-0' } });
        case 'media_board_set_bindings':
          return Promise.resolve({ ok: true, bindings: payload.bindings || {}, failedAccelerators: [] });
        case 'media_board_trigger':
          return delay(250, trigger(payload.itemId));
        case 'media_board_stop':
          finishPlaying();
          return Promise.resolve({ ok: true });
        case 'media_board_stop_local':
          // Like the shell: only this device's overlay is cleared, the guild
          // keeps playing (the queue is untouched). A second stop of the same
          // job finds nothing on screen.
          if (!queue.playing || queue.playing.id === locallyStoppedJobId) {
            return Promise.resolve({ ok: false, reason: 'nothing_playing' });
          }
          locallyStoppedJobId = queue.playing.id;
          return Promise.resolve({ ok: true });
        case 'guild_select':
          guilds.activeGuildId = payload.guildId;
          guilds.guilds.forEach((guild) => {
            guild.membership.active = guild.membership.guildId === payload.guildId;
          });
          return Promise.resolve({ ok: true, guilds: guildsCopy() });
        case 'guild_set_muted':
          guilds.guilds.forEach((guild) => {
            if (guild.membership.guildId === payload.guildId) {
              guild.membership.muted = Boolean(payload.muted);
            }
          });
          if (settings.guilds[payload.guildId]) {
            settings.guilds[payload.guildId].muted = Boolean(payload.muted);
          }
          applyMuted(payload.guildId, Boolean(payload.muted));
          return Promise.resolve({ ok: true, guilds: guildsCopy() });
        case 'guild_leave':
          return Promise.resolve({ ok: true, guilds: guildsCopy() });
        case 'pairing_consume': {
          const code = String(payload.code || '').toUpperCase();
          if (code === 'NOPSEUDO' && !String(payload.displayName || '').trim()) {
            return Promise.resolve({ ok: false, message: 'appairage refusé (HTTP 400 - {"error":"display_name_required"})' });
          }
          return Promise.resolve({ ok: false, message: 'appairage refusé (HTTP 404 - {"error":"pairing_code_invalid_or_expired"})' });
        }
        default:
          return Promise.resolve(null);
      }
    },
  };
})();
