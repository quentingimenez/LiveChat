// Harnais de développement : simule le shell Tauri pour afficher l'application
// principale (`web/board.html`) dans un navigateur ordinaire, sans serveur.
// Chargé par `dev/board-mock.html` avant `web/app/app.js`. Jamais embarqué dans
// l'application (le dossier `dev/` est hors de `frontendDist`).
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

  const guilds = {
    activeGuildId: empty ? null : '111',
    enabled: true,
    profile: { displayName: 'Quentin', avatar: null },
    guilds: empty
      ? []
      : [
          { membership: membership('111', 'Les Sardines', { active: true }), connection: connection('connected'), statusLabel: 'connecté', playing: true },
          { membership: membership('222', 'GG Squad'), connection: connection('reconnecting'), statusLabel: 'reconnexion…', playing: false },
          { membership: membership('333', 'TV du salon', { muted: true }), connection: connection('disabled'), statusLabel: 'désactivé', playing: false },
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
    addMeme: !isGuest,
    editAnyMeme: isAdmin,
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
    if (path.startsWith('/overlay/meme-board/items') && method === 'GET') {
      return ok({ total: items.length, items });
    }
    if (path === '/overlay/queue' && method === 'GET') {
      return ok(queue);
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

  window.__TAURI_INTERNALS__ = {
    transformCallback: () => 1,
    invoke(command, args) {
      const payload = (args && args.payload) || {};
      switch (command) {
        case 'guilds_list':
          return Promise.resolve(guilds);
        case 'guild_api':
          return Promise.resolve(api(payload.method, payload.path));
        case 'guild_media_url':
          return Promise.resolve(svg(Number(String(payload.assetId).slice(1)) || 0));
        case 'meme_board_get_bindings':
          return Promise.resolve({ bindings: { 'CommandOrControl+Shift+1': 'item-0' } });
        case 'meme_board_set_bindings':
          return Promise.resolve({ ok: true, bindings: payload.bindings || {}, failedAccelerators: [] });
        case 'meme_board_trigger':
        case 'meme_board_stop':
          return Promise.resolve({ ok: true });
        case 'guild_select':
        case 'guild_set_muted':
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
