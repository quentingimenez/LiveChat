// Harnais de développement : simule le shell Tauri pour afficher l'application
// principale (`web/board.html`) dans un navigateur ordinaire, sans serveur.
// Chargé par `dev/board-mock.html` avant `web/app/main.js`. Jamais embarqué dans
// l'application (le dossier `dev/` est hors de `frontendDist`).
//
// Options de l'URL : ?role=admin|member|guest, ?empty (aucune guilde),
// ?removed (un retrait de guilde en attente au démarrage).
// Depuis la console : `__mock.emit(event, payload)`,
// `__mock.removeGuild(guildId, reason)`, `__mock.finishPlaying()`.
// Un fichier dont le nom contient « fail » est refusé (415), « big » (413).
// « Sad violin » n'est pas prêt (media_not_ready).
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

  function guildIcon(hue) {
    return 'data:image/svg+xml,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" fill="hsl(${hue} 55% 45%)"/><path d="M34 88 64 34l30 54z" fill="hsl(${hue} 80% 85%)"/></svg>`);
  }

  const guilds = {
    activeGuildId: empty ? null : '111',
    enabled: true,
    profile: { displayName: 'Quentin', avatar: null },
    guilds: empty
      ? []
      : [
          { membership: membership('111', 'Les Sardines', { active: true }), connection: connection('connected'), statusLabel: 'connecté', playing: true },
          { membership: membership('222', 'GG Squad', { guildIconUrl: guildIcon(200) }), connection: connection('reconnecting'), statusLabel: 'reconnexion…', playing: false },
          { membership: membership('333', 'TV du salon', { muted: true, guildIconUrl: guildIcon(30) }), connection: connection('disabled'), statusLabel: 'désactivé', playing: false },
          // An icon that fails to load: the rail falls back to the initials.
          { membership: membership('444', 'L’Escale 🔱', { guildIconUrl: 'https://cdn.discordapp.com/icons/0/missing.png' }), connection: connection('connected'), statusLabel: 'connecté', playing: false },
        ],
  };

  const titles = ['Chat qui tombe', 'Bruh', 'Vine boom', 'Directed by Robert B. Weide', 'Stonks', 'Ah shit, here we go again', null, 'Fanfare de la victoire totale et définitive', 'Nope', 'Sad violin'];
  const kinds = ['video', 'image', 'audio', 'video', 'image', 'video', 'video', 'audio', 'image', 'audio'];
  const items = titles.map((title, index) => ({
    id: 'item-' + index,
    guildId: '111',
    mediaAssetId: 'a' + index,
    title,
    message: index === 0 ? 'aïe' : null,
    createdByName: index % 2 ? 'Léa' : 'Quentin',
    createdByMemberId: index % 2 ? 'm-lea' : 'm-111',
    canEdit: isAdmin || (!isGuest && index % 2 === 0),
    createdAt: new Date(Date.now() - index * 86400000).toISOString(),
    media: { id: 'a' + index, kind: kinds[index], mime: 'x', durationSec: kinds[index] === 'image' ? null : 3 + index * 7, width: 1280, height: 720, isVertical: false, sizeBytes: 420000 * (index + 1), sourceUrl: 'https://www.tiktok.com/@x/video/' + index },
  }));

  const now = Date.now();
  const queue = {
    playing: { id: 'j0', status: 'playing', title: 'Chat qui tombe', text: null, authorName: 'Léa', durationSec: 30, kind: 'video', startedAt: new Date(now - 12000).toISOString(), submittedAt: new Date(now - 20000).toISOString() },
    pending: [
      { id: 'j1', status: 'pending', title: 'Stonks', authorName: 'Max', durationSec: 10, kind: 'image' },
      { id: 'j2', status: 'pending', title: null, text: 'gg les gars', authorName: 'Léa', durationSec: 5, kind: null },
      { id: 'j3', status: 'pending', title: 'Directed by Robert B. Weide', authorName: 'Invité-42', durationSec: 8, kind: 'video' },
    ],
  };

  const members = [
    { id: 'm-111', displayName: 'Quentin', avatar: null, role: 'admin', connected: true, deviceCount: 2, lastSeenAt: new Date().toISOString(), isSelf: true },
    { id: 'm-lea', displayName: 'Léa', avatar: null, role: 'member', connected: true, deviceCount: 1, lastSeenAt: new Date().toISOString(), isSelf: false },
    { id: 'm-max', displayName: 'Max', avatar: null, role: 'member', connected: false, deviceCount: 1, lastSeenAt: new Date(now - 7200000).toISOString(), isSelf: false },
    { id: 'm-42', displayName: 'Invité-42', avatar: null, role: 'guest', connected: false, deviceCount: 1, lastSeenAt: null, isSelf: false },
  ];
  const invites = [
    { code: 'K7QXM2PD', role: 'member', maxUses: 1, useCount: 0, expiresAt: new Date(now + 22 * 3600000).toISOString(), revokedAt: null, isActive: true },
    { code: 'P3WN8RTA', role: 'guest', maxUses: 10, useCount: 4, expiresAt: new Date(now + 6 * 86400000).toISOString(), revokedAt: null, isActive: true },
    { code: 'Z9LCV5HE', role: 'admin', maxUses: 1, useCount: 1, expiresAt: null, revokedAt: null, isActive: false },
  ];

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
    const hue = (index * 47) % 360;
    return 'data:image/svg+xml,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="hsl(${hue} 35% 32%)"/><circle cx="${60 + index * 20}" cy="100" r="46" fill="hsl(${hue} 45% 52%)"/></svg>`);
  }

  function api(method, path) {
    const ok = (body) => ({ ok: true, status: 200, body });
    if (path === '/overlay/guild') {
      return ok({
        guild: { id: '111', name: 'Les Sardines', standalone: true, discordGuildId: null, guestsCanViewBoard: true, settings: { defaultMediaTime: 30, maxMediaTime: null, displayMediaFull: false }, connectedCount: 6 },
        member: { id: 'm-111', displayName: 'Quentin', avatar: null, role },
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
    if (path === '/overlay/admin/invites' && method === 'GET') {
      return ok({ count: invites.length, items: invites });
    }
    if (path === '/overlay/device-code') {
      return ok({ created: true, invite: { code: 'D3V1C3AB' } });
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
    const job = { id: 'j' + nextJob++, title: item.title, text: item.message, authorName: 'Quentin', durationSec: item.media.durationSec || 8, kind: item.media.kind };
    queueHint();
    if (!queue.playing) {
      const at = new Date().toISOString();
      queue.playing = Object.assign(job, { status: 'playing', startedAt: at, submittedAt: at });
      return { ok: true, outcome: 'playing', jobId: job.id };
    }
    queue.pending.push(Object.assign(job, { status: 'pending' }));
    return { ok: true, outcome: 'queued', position: queue.pending.length, jobId: job.id };
  }
  function finishPlaying() {
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
  function finishUpload(upload) {
    const name = String(upload.filename || '').toLowerCase();
    if (name.includes('big')) {
      return { ok: false, status: 413, body: { error: 'payload_too_large' } };
    }
    if (name.includes('fail')) {
      return { ok: false, status: 415, body: { error: 'unsupported_media' } };
    }
    const index = items.length;
    const kind = String(upload.contentType || '').split('/')[0];
    const item = {
      id: 'item-' + index,
      guildId: upload.guildId || '111',
      mediaAssetId: 'a' + index,
      title: upload.title || null,
      message: upload.message || null,
      createdByName: 'Quentin',
      createdByMemberId: 'm-111',
      canEdit: true,
      createdAt: new Date().toISOString(),
      media: { id: 'a' + index, kind: ['video', 'image', 'audio'].includes(kind) ? kind : 'video', mime: upload.contentType || 'x', durationSec: kind === 'image' ? null : 6, width: 1280, height: 720, isVertical: false, sizeBytes: upload.received, sourceUrl: null },
    };
    items.unshift(item);
    return { ok: true, status: 200, body: { created: true, item } };
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
    emit('overlay://guilds', guilds);
  }

  window.__mock = { emit, removeGuild, finishPlaying, queue, items, guilds };

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
          return upload ? delay(1500, finishUpload(upload)) : Promise.reject(new Error('upload_not_found'));
        }
        case 'media_upload_cancel':
          return Promise.resolve(uploads.delete(payload.uploadId));
        case 'board_take_guild_removals':
          return Promise.resolve(removals.splice(0));
        case 'guilds_list':
          return Promise.resolve(guilds);
        case 'guild_api':
          return Promise.resolve(api(payload.method, payload.path));
        case 'guild_media_url':
          return Promise.resolve(svg(Number(String(payload.assetId).slice(1)) || 0));
        case 'media_board_get_bindings':
          return Promise.resolve({ bindings: { 'CommandOrControl+Shift+1': 'item-0' } });
        case 'media_board_set_bindings':
          return Promise.resolve({ ok: true, bindings: payload.bindings || {}, failedAccelerators: [] });
        case 'media_board_trigger':
          return delay(250, trigger(payload.itemId));
        case 'media_board_stop':
          finishPlaying();
          return Promise.resolve({ ok: true });
        case 'guild_select':
          guilds.activeGuildId = payload.guildId;
          guilds.guilds.forEach((guild) => {
            guild.membership.active = guild.membership.guildId === payload.guildId;
          });
          return Promise.resolve({ ok: true, guilds });
        case 'guild_set_muted':
          guilds.guilds.forEach((guild) => {
            if (guild.membership.guildId === payload.guildId) {
              guild.membership.muted = Boolean(payload.muted);
            }
          });
          return Promise.resolve({ ok: true, guilds });
        case 'guild_leave':
          return Promise.resolve({ ok: true, guilds });
        case 'pairing_consume':
          return Promise.resolve({ ok: false, message: 'appairage refusé (HTTP 404 - {"error":"pairing_code_invalid_or_expired"})' });
        default:
          return Promise.resolve(null);
      }
    },
  };
})();
