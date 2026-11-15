// LiveChat admin console — static mockup. No backend: every action mutates the
// fake data below and shows a toast. Plain script (no modules) so the page
// opens by double-click from disk.
(function () {
  'use strict';

  const BOT_INVITE_URL =
    'https://discord.com/oauth2/authorize?client_id=1570564356108714094&scope=bot%20applications.commands&permissions=19456';
  const MIN_OVERLAY_VERSION = '3.2.0';
  const GIB = 1024 * 1024 * 1024;
  const MIB = 1024 * 1024;
  const BOARD_MAX = 15 * GIB;
  const CACHE_MAX = 5 * GIB;

  // ------------------------------------------------------------------ helpers
  const esc = (value) =>
    String(value ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  const ic = (name) => `<svg class="ic" aria-hidden="true"><use href="#i-${name}"/></svg>`;
  const plural = (count, one, many) => `${count} ${count > 1 ? many : one}`;

  function bytes(value) {
    if (value >= GIB) {
      return `${(value / GIB).toFixed(1).replace('.', ',')} Go`;
    }
    return `${Math.round(value / MIB)} Mo`;
  }

  function ago(minutes) {
    if (minutes == null) {
      return 'jamais';
    }
    if (minutes < 1) {
      return 'à l’instant';
    }
    if (minutes < 60) {
      return `il y a ${minutes} min`;
    }
    if (minutes < 60 * 48) {
      return `il y a ${Math.round(minutes / 60)} h`;
    }
    return `il y a ${Math.round(minutes / 1440)} j`;
  }

  function versionBelow(version, minimum) {
    const a = version.split('.').map(Number);
    const b = minimum.split('.').map(Number);
    for (let i = 0; i < 3; i += 1) {
      if (a[i] !== b[i]) {
        return a[i] < b[i];
      }
    }
    return false;
  }

  function meter(used, max, label) {
    const percent = Math.min(100, Math.round((used / max) * 100));
    return `<div class="meter${percent >= 85 ? ' is-high' : ''}" role="img" aria-label="${esc(label || 'Occupation')} : ${percent} %"><span style="width:${percent}%"></span></div>`;
  }

  const initials = (name) => name.replace(/[^\p{L}\p{N} ]/gu, '').split(' ').filter(Boolean).slice(0, 2).map((part) => part[0].toUpperCase()).join('');
  const who = (name) => `<span class="who"><span class="avatar" aria-hidden="true">${esc(initials(name))}</span>${esc(name)}</span>`;

  // --------------------------------------------------------------- fake data
  // Seeded generator: the same realistic data on every load.
  let seed = 20261115;
  function rand() {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  const pick = (list) => list[Math.floor(rand() * list.length)];
  const int = (min, max) => min + Math.floor(rand() * (max - min + 1));
  const hex = (length) => Array.from({ length }, () => '0123456789abcdef'[int(0, 15)]).join('');
  const uuid = () => `${hex(8)}-${hex(4)}-4${hex(3)}-a${hex(3)}-${hex(12)}`;
  const code = () => Array.from({ length: 8 }, () => 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789'[int(0, 31)]).join('');
  const snowflake = () => `${int(1, 9)}${Array.from({ length: 17 }, () => int(0, 9)).join('')}`;

  const PEOPLE = ['Quentin', 'Maëlle', 'Théo', 'Inès', 'Bastien', 'Clara', 'Nassim', 'Lucie', 'Hugo', 'Yanis', 'Camille', 'Romane', 'Élise', 'Malo', 'Sacha', 'Jade', 'Antoine', 'Lina', 'Gabin', 'Zoé'];
  const MACHINES = ['PC-Salon', 'PC-Bureau', 'Tour-Gaming', 'Portable', 'PC-Stream', 'Mini-PC-TV'];
  const MEDIA = [
    ['Le chat qui tombe du canapé', 'video', 14],
    ['Airhorn', 'audio', 3],
    ['Ah !', 'audio', 2],
    ['Nelson — Ha-ha', 'audio', 2],
    ['Le fail du dernier rond', 'video', 22],
    ['GG les gars', 'image', 5],
    ['Bruit de moteur douteux', 'audio', 6],
    ['Clip TikTok — danse du pingouin', 'video', 17],
    ['Trombone triste', 'audio', 4],
    ['Mème « c’est honteux »', 'image', 5],
    ['Applaudissements', 'audio', 7],
    ['Replay — le clutch de Théo', 'video', 31],
  ];
  const KIND_LABEL = { video: 'Vidéo', audio: 'Son', image: 'Image', text: 'Texte' };

  function buildGuild(spec) {
    const names = PEOPLE.slice().sort(() => rand() - 0.5).slice(0, spec.members);
    if (spec.owner && !names.includes(spec.owner)) {
      names[0] = spec.owner;
    }
    names.sort((a, b) => (a === spec.owner ? -1 : b === spec.owner ? 1 : 0));
    const members = names.map((name, index) => ({
      id: uuid(),
      name,
      role: index === 0 ? 'admin' : index === 1 && spec.members > 5 ? 'admin' : rand() < 0.22 ? 'guest' : 'member',
      discordUserId: spec.kind === 'none' && rand() < 0.6 ? null : snowflake(),
      joinedDays: int(3, 320),
      lastSeen: null,
    }));
    const devices = [];
    members.forEach((member, index) => {
      const count = index < spec.devices ? (rand() < 0.2 ? 2 : 1) : 0;
      for (let i = 0; i < count; i += 1) {
        const connected = spec.live && rand() < 0.62;
        const lastSeen = connected ? 0 : pick([18, 95, 400, 1500, 4300, 12000]);
        devices.push({
          id: uuid(),
          label: `${pick(MACHINES)}-${member.name}`,
          memberId: member.id,
          sessionMode: member.role === 'guest' ? 'invite_read_only' : 'normal',
          connected,
          muted: !connected && rand() < 0.15,
          lastSeen,
          createdDays: int(1, member.joinedDays),
          version: pick(['3.3.0', '3.3.0', '3.3.0', '3.2.0', '3.1.2']),
          os: pick(['Windows 11', 'Windows 11', 'Windows 10', 'macOS 15']),
        });
        member.lastSeen = member.lastSeen == null ? lastSeen : Math.min(member.lastSeen, lastSeen);
      }
    });
    const admins = members.filter((member) => member.role === 'admin');
    const invites = Array.from({ length: spec.invites }, (_, index) => {
      const maxUses = pick([1, 10, null]);
      const status = index === 0 ? 'active' : pick(['active', 'expired', 'revoked', 'used']);
      return {
        code: code(),
        role: pick(['member', 'member', 'guest', 'admin']),
        createdBy: pick(admins).name,
        createdMin: int(20, 9000),
        expiresHours: status === 'active' ? pick([6, 19, 96, null]) : null,
        maxUses,
        useCount: status === 'used' ? maxUses || 1 : maxUses === 1 ? 0 : int(0, 4),
        status,
      };
    });
    const pairing = Array.from({ length: spec.pairing }, (_, index) => ({
      code: code(),
      authorName: pick(members).name,
      mode: rand() < 0.3 ? 'invite_read_only' : 'normal',
      createdMin: int(2, 900),
      status: index === 0 ? 'active' : pick(['active', 'used', 'expired']),
      expiresMin: int(1, 9),
    }));
    const ingest = Array.from({ length: spec.ingest }, () => {
      const author = pick(members);
      return { id: uuid(), label: `${pick(['Chrome', 'Firefox', 'Edge'])}-${author.name}`, authorName: author.name, lastSeen: pick([4, 60, 700, 5000, null]), createdDays: int(2, 200) };
    });
    const board = {
      items: spec.board,
      usedBytes: Math.round(spec.boardGb * GIB),
      kinds: { video: Math.round(spec.board * 0.46), audio: Math.round(spec.board * 0.38), image: spec.board - Math.round(spec.board * 0.46) - Math.round(spec.board * 0.38) },
      favorites: Math.round(spec.board * 0.12),
      recent: spec.board
        ? MEDIA.slice().sort(() => rand() - 0.5).slice(0, 6).map(([title, kind, duration]) => ({
            title,
            kind,
            duration,
            by: pick(members).name,
            size: kind === 'video' ? int(4, 48) * MIB : int(1, 3) * MIB,
            plays: int(0, 140),
            addedMin: int(30, 30000),
          }))
        : [],
    };
    const queue = spec.onAir
      ? MEDIA.slice().sort(() => rand() - 0.5).slice(0, spec.onAir).map(([title, kind, duration], index) => ({
          id: uuid(),
          title,
          kind,
          duration,
          by: pick(members).name,
          source: pick(['Board', 'Discord /send', 'Extension']),
          status: index === 0 ? 'playing' : 'pending',
          progress: index === 0 ? int(20, 80) : 0,
        }))
      : [];
    return {
      id: spec.id,
      name: spec.name,
      kind: spec.kind,
      discordServerId: spec.kind === 'native' ? spec.id : spec.kind === 'linked' ? snowflake() : null,
      discordName: spec.discordName || null,
      botPresent: spec.botPresent !== false && spec.kind !== 'none',
      createdDays: spec.createdDays,
      lastActivity: spec.lastActivity,
      settings: { defaultMediaTime: spec.defaultMediaTime ?? null, maxMediaTime: spec.maxMediaTime ?? null, displayMediaFull: Boolean(spec.full), guestsCanViewBoard: spec.guestsBoard !== false },
      members,
      devices,
      invites,
      pairing,
      ingest,
      board,
      queue,
      playback: { pending: Math.max(0, queue.length - 1), playing: queue.length ? 1 : 0, failed: int(0, 6), done: int(40, 2400) },
    };
  }

  const guilds = [
    { id: '812345678901234567', name: 'Les Zigotos du Dimanche', kind: 'native', members: 14, devices: 9, live: true, onAir: 4, invites: 5, pairing: 3, ingest: 3, board: 212, boardGb: 9.4, createdDays: 301, lastActivity: 0, owner: 'Quentin', defaultMediaTime: 6, maxMediaTime: 45 },
    { id: '734410022918876160', name: 'Stream de Maëlle', kind: 'native', members: 6, devices: 4, live: true, onAir: 2, invites: 2, pairing: 1, ingest: 1, board: 88, boardGb: 13.1, createdDays: 164, lastActivity: 1, owner: 'Maëlle', full: true },
    { id: '900000000000000001', name: 'Soirée Canap’', kind: 'linked', discordName: 'Canap’ & Chill', members: 8, devices: 5, live: true, onAir: 0, invites: 3, pairing: 0, ingest: 2, board: 57, boardGb: 2.2, createdDays: 92, lastActivity: 34, owner: 'Théo', maxMediaTime: 30 },
    { id: '900000000000000002', name: 'LAN des cousins', kind: 'none', members: 5, devices: 5, live: false, onAir: 0, invites: 4, pairing: 0, ingest: 0, board: 23, boardGb: 0.6, createdDays: 37, lastActivity: 2900, owner: 'Bastien', guestsBoard: false },
    { id: '698812004417552384', name: 'Team Rocket Ligue', kind: 'native', members: 11, devices: 6, live: true, onAir: 0, invites: 1, pairing: 2, ingest: 1, board: 131, boardGb: 5.8, createdDays: 248, lastActivity: 190, owner: 'Inès' },
    { id: '900000000000000003', name: 'Coloc du 12 bis', kind: 'none', members: 4, devices: 3, live: false, onAir: 0, invites: 1, pairing: 0, ingest: 1, board: 41, boardGb: 1.1, createdDays: 58, lastActivity: 760, owner: 'Clara' },
    { id: '655190873322110976', name: '', kind: 'native', botPresent: false, members: 2, devices: 1, live: false, onAir: 0, invites: 0, pairing: 1, ingest: 0, board: 6, boardGb: 0.1, createdDays: 410, lastActivity: 61000, owner: 'Hugo' },
  ].map(buildGuild);

  const guildName = (guild) => guild.name || `Guild ${guild.id}`;
  const findGuild = (id) => guilds.find((guild) => guild.id === id);
  const memberOf = (guild, id) => guild.members.find((member) => member.id === id);
  const allDevices = () => guilds.flatMap((guild) => guild.devices.map((device) => ({ device, guild })));

  const server = {
    version: '3.3.0',
    uptime: '6 j 4 h',
    cacheUsed: Math.round(3.7 * GIB),
    ingestions: [
      { title: 'Clip TikTok — danse du pingouin', guild: guilds[0], by: 'Extension (Chrome-Quentin)', status: 'processing', detail: 'Transcodage 62 %', min: 0 },
      { title: 'https://x.com/…/status/19873…', guild: guilds[1], by: 'Discord /send — Maëlle', status: 'failed', detail: 'Vidéo introuvable (resolver Twitter)', min: 42 },
      { title: 'TikTok restreint (connexion requise)', guild: guilds[0], by: 'Board — Théo', status: 'failed', detail: 'Cookie TikTok requis', min: 180 },
      { title: 'Replay — le clutch de Théo', guild: guilds[4], by: 'Board — Inès', status: 'ready', detail: '31 s, 38 Mo, 1080p', min: 195 },
      { title: 'fichier_3go.mkv', guild: guilds[2], by: 'Board — Théo', status: 'failed', detail: 'Dépasse MEDIA_MAX_SIZE_MB (200)', min: 800 },
      { title: 'Trombone triste', guild: guilds[5], by: 'Board — Clara', status: 'ready', detail: '4 s, 1 Mo', min: 760 },
    ],
  };

  const activity = [
    [2, 'quentin', 'Arrêt de la lecture', guilds[2], 'POST /admin/api/guilds/{id}/stop', 200],
    [26, 'quentin', 'Overlay révoqué — Portable-Yanis', guilds[0], 'POST /admin/api/overlay-clients/{id}/revoke', 200],
    [31, 'quentin', 'Réglages de guilde modifiés', guilds[0], 'PATCH /admin/api/guilds/{id}/settings', 200],
    [140, 'maelle', 'Client d’ingestion créé — Chrome-Maëlle', guilds[1], 'POST /admin/api/ingest-clients', 200],
    [320, 'quentin', 'Codes d’appairage expirés purgés (7)', null, 'DELETE /admin/api/pairing-codes/expired', 200],
    [1300, 'secret admin', 'Cookie TikTok mis à jour', null, 'PATCH /admin/api/runtime-settings', 200],
    [1420, 'inconnu (203.0.113.42)', 'Connexion refusée', null, 'GET /admin/api/overview', 401],
    [2900, 'quentin', 'Guilde créée — Coloc du 12 bis', guilds[5], 'POST /admin/api/guilds', 200],
    [4400, 'quentin', 'Board média vidé', guilds[6], 'POST /admin/api/guilds/{id}/media-board/clear', 200],
    [7100, 'maelle', 'Purge refusée (confirmation erronée)', guilds[1], 'POST /admin/api/guilds/{id}/purge', 400],
  ].map(([min, actor, action, guild, route, status]) => ({ min, actor, action, guild, route, status, requestId: hex(8) }));

  const ROLE_LABEL = { admin: 'Admin', member: 'Membre', guest: 'Invité' };
  const MODE_LABEL = { normal: 'Contrôle complet', invite_read_only: 'Lecture seule' };
  const KIND_BADGE = {
    native: ['Serveur Discord', 'is-info'],
    linked: ['Indépendante, liée', 'is-info'],
    none: ['Indépendante', 'is-outline'],
  };
  // Mirrors `Permission` / `GuildRole::can` in crates/lc-domain/src/enums.rs.
  const PERMISSIONS = [
    ['Recevoir les médias', 'guest'],
    ['Voir la file d’attente', 'guest'],
    ['Consulter le board', 'board'],
    ['Lancer une lecture', 'member'],
    ['Arrêter la lecture', 'member'],
    ['Ajouter des médias', 'member'],
    ['Modifier ses médias', 'member'],
    ['Modifier tous les médias', 'admin'],
    ['Modérer la file', 'admin'],
    ['Gérer les membres', 'admin'],
    ['Gérer les invitations', 'admin'],
    ['Modifier les réglages', 'admin'],
    ['Purger la guilde', 'admin'],
  ];

  function permissionGranted(level, role, mode, guestsCanViewBoard) {
    const effective = mode === 'invite_read_only' ? 'guest' : role;
    if (level === 'guest') {
      return true;
    }
    if (level === 'board') {
      return effective !== 'guest' || guestsCanViewBoard;
    }
    if (level === 'member') {
      return effective !== 'guest';
    }
    return effective === 'admin';
  }

  // -------------------------------------------------------------- app state
  const state = {
    user: null,
    filters: { guildQuery: '', guildKind: 'all', deviceQuery: '', deviceState: 'all', codeStatus: 'all', logQuery: '' },
  };
  try {
    state.user = JSON.parse(sessionStorage.getItem('lc-admin-mock-user') || 'null');
  } catch (error) {
    state.user = null;
  }
  function setUser(user) {
    state.user = user;
    try {
      if (user) {
        sessionStorage.setItem('lc-admin-mock-user', JSON.stringify(user));
      } else {
        sessionStorage.removeItem('lc-admin-mock-user');
      }
    } catch (error) {
      // Storage may be unavailable from file://: the session then lasts one page load.
    }
  }

  const app = document.getElementById('app');
  const dialog = document.getElementById('dialog');
  const toasts = document.getElementById('toasts');
  let regionRenderer = null;
  let dialogSubmit = null;

  function toast(message) {
    const node = document.createElement('div');
    node.className = 'toast';
    node.innerHTML = `${ic('check')}<span>${esc(message)}</span>`;
    toasts.append(node);
    setTimeout(() => node.remove(), 3600);
  }

  function openDialog(html, onSubmit, wide) {
    dialog.className = wide ? 'is-wide' : '';
    dialog.innerHTML = html;
    dialogSubmit = onSubmit || null;
    if (!dialog.open) {
      dialog.showModal();
    }
  }

  function dialogShell({ title, sub, body, foot }) {
    return `<form method="dialog" data-dialog-form>
      <div class="dlg-head"><div><h2 id="dialog-title">${esc(title)}</h2>${sub ? `<p>${sub}</p>` : ''}</div>
        <button type="button" class="btn is-quiet is-icon" data-act="close" aria-label="Fermer">${ic('x')}</button></div>
      <div class="dlg-body">${body}</div>
      <div class="dlg-foot">${foot}</div>
    </form>`;
  }

  // Confirmation of a destructive action. `expect` asks to type a value first,
  // like `confirmGuildId` of the existing purge / revoke routes.
  function confirmDialog({ title, body, label, expect, extra, onConfirm }) {
    const typed = expect
      ? `<div class="dlg-section"><label for="confirm-text">Pour confirmer, saisis l’identifiant <span class="mono">${esc(expect)}</span></label>
         <input class="field mono" id="confirm-text" autocomplete="off" spellcheck="false" style="margin-top:6px" data-expect="${esc(expect)}"></div>`
      : '';
    openDialog(
      dialogShell({
        title,
        body: `<p class="soft">${body}</p>${extra || ''}${typed}`,
        foot: `<button type="button" class="btn" data-act="close">Annuler</button>
               <button type="submit" class="btn is-danger-solid" id="confirm-go"${expect ? ' disabled' : ''}>${esc(label)}</button>`,
      }),
      (form) => {
        dialog.close();
        onConfirm(form);
      },
    );
  }

  // ---------------------------------------------------------------- routing
  function route() {
    const parts = location.hash.replace(/^#\/?/, '').split('/').filter(Boolean).map(decodeURIComponent);
    return parts.length ? parts : ['dashboard'];
  }

  const NAV = [
    ['Serveur', [
      ['dashboard', 'Tableau de bord', 'dashboard'],
      ['guilds', 'Guildes', 'guilds', () => guilds.length],
      ['devices', 'Overlays', 'device', () => allDevices().length],
    ]],
    ['Accès', [
      ['codes', 'Codes et invitations', 'key'],
      ['ingest', 'Clients d’ingestion', 'plug', () => guilds.reduce((sum, guild) => sum + guild.ingest.length, 0)],
    ]],
    ['Exploitation', [
      ['media', 'Médias et stockage', 'disk'],
      ['activity', 'Journal', 'log'],
      ['settings', 'Paramètres du serveur', 'settings'],
    ]],
  ];

  function shell(current, crumbs, content) {
    const nav = NAV.map(
      ([group, links]) =>
        `<p class="nav-group">${group}</p>` +
        links
          .map(
            ([id, label, icon, count]) =>
              `<a href="#/${id}" title="${esc(label)}"${current === id ? ' aria-current="page"' : ''}>${ic(icon)}<span>${esc(label)}</span>${count ? `<span class="count">${count()}</span>` : ''}</a>`,
          )
          .join(''),
    ).join('');
    const connected = allDevices().filter(({ device }) => device.connected).length;
    return `<div class="shell">
      <aside class="side">
        <a class="brand" href="#/dashboard" aria-label="LiveChat, tableau de bord"><svg aria-hidden="true"><use href="#i-logo"/></svg><span>LiveChat<small>Administration</small></span></a>
        <nav class="nav" aria-label="Navigation principale">${nav}</nav>
        <div class="side-foot">
          <a class="btn" href="${BOT_INVITE_URL}" target="_blank" rel="noopener" title="Inviter le bot sur un serveur Discord">${ic('plus')}<span>Inviter le bot</span></a>
          <p>Serveur ${server.version} · livechat.example.net</p>
        </div>
      </aside>
      <div class="main">
        <header class="top">
          <div class="crumbs">${crumbs}</div>
          <span class="spacer"></span>
          <span class="mock-flag">Maquette — données fictives</span>
          <span class="health-pill"><span class="dot is-ok"></span>Opérationnel<span class="long"> · ${connected} overlays en ligne</span></span>
          <span class="user" title="${esc(state.user.via)}"><span class="avatar" aria-hidden="true">${esc(initials(state.user.name))}</span><span class="long">${esc(state.user.name)}</span></span>
          <button class="btn is-quiet is-icon" data-act="logout" aria-label="Se déconnecter" title="Se déconnecter">${ic('logout')}</button>
        </header>
        <main class="page" id="content" tabindex="-1">${content}</main>
      </div>
    </div>`;
  }

  const crumb = (...items) => items.map((item, index) => (index === items.length - 1 ? `<strong>${esc(item[0])}</strong>` : `<a href="${item[1]}">${esc(item[0])}</a>${ic('chevron')}`)).join('');

  function render() {
    const parts = route();
    regionRenderer = null;
    if (!state.user || parts[0] === 'login') {
      if (state.user) {
        location.hash = '#/dashboard';
        return;
      }
      app.innerHTML = loginPage();
      document.title = 'Connexion — LiveChat Administration';
      return;
    }
    const pages = { dashboard, guilds: guildsPage, devices: devicesPage, codes: codesPage, ingest: ingestPage, media: mediaPage, activity: activityPage, settings: settingsPage };
    let page;
    if (parts[0] === 'guilds' && parts[1]) {
      page = guildDetail(parts[1], parts[2] || 'overview');
    } else {
      page = (pages[parts[0]] || dashboard)();
    }
    app.innerHTML = shell(page.nav, page.crumbs, page.html);
    document.title = `${page.title} — LiveChat Administration`;
    if (regionRenderer) {
      refreshRegion();
    }
  }

  function refreshRegion() {
    const region = document.getElementById('region');
    if (region && regionRenderer) {
      region.innerHTML = regionRenderer();
    }
  }

  // ------------------------------------------------------------------ login
  function loginPage() {
    return `<div class="login">
      <section class="login-aside">
        <div class="brand"><svg aria-hidden="true"><use href="#i-logo"/></svg><span>LiveChat<small>Administration du serveur</small></span></div>
        <div>
          <h1>La régie du serveur.</h1>
          <p class="lede">Guildes, overlays, accès, médias et réglages de toute l’instance. Réservé aux administrateurs du serveur LiveChat.</p>
        </div>
        <dl class="login-facts">
          <div><dt>Instance</dt><dd>livechat.example.net</dd></div>
          <div><dt>Version</dt><dd>${server.version}</dd></div>
          <div><dt>État</dt><dd>opérationnel</dd></div>
        </dl>
      </section>
      <main class="login-main" id="content">
        <div class="login-box">
          <h2>Connexion</h2>
          <p>Connecte-toi avec le compte Discord déclaré comme administrateur de ce serveur.</p>
          <button class="btn-discord" data-act="login-discord"><svg aria-hidden="true"><use href="#i-discord"/></svg>Se connecter avec Discord</button>
          <div class="or">ou</div>
          <form data-form="login-secret">
            <label for="secret">Secret d’administration</label>
            <input class="field" id="secret" type="password" autocomplete="current-password" placeholder="ADMIN_UI_TOKEN" style="margin-top:6px">
            <p class="hint">Accès de secours, sans Discord : la valeur de <span class="mono">ADMIN_UI_TOKEN</span> du serveur.</p>
            <button class="btn" type="submit" style="margin-top:12px;width:100%;justify-content:center">Se connecter avec le secret</button>
          </form>
          <p class="login-note">${ic('shield')}<span>Session de 12 h dans un cookie <span class="mono">HttpOnly</span>. Le secret n’est jamais stocké dans le navigateur ni placé dans une URL.</span></p>
        </div>
      </main>
    </div>`;
  }

  // -------------------------------------------------------------- dashboard
  function dashboard() {
    const devices = allDevices();
    const connected = devices.filter(({ device }) => device.connected).length;
    const outdated = devices.filter(({ device }) => versionBelow(device.version, MIN_OVERLAY_VERSION)).length;
    const boardUsed = guilds.reduce((sum, guild) => sum + guild.board.usedBytes, 0);
    const activeCodes = guilds.reduce((sum, guild) => sum + guild.pairing.filter((item) => item.status === 'active').length + guild.invites.filter((item) => item.status === 'active').length, 0);
    const failed = server.ingestions.filter((item) => item.status === 'failed').length;
    const onAir = guilds.filter((guild) => guild.queue.length);
    const fullest = guilds.slice().sort((a, b) => b.board.usedBytes - a.board.usedBytes).slice(0, 4);

    const air = onAir.length
      ? onAir
          .map((guild) => {
            const job = guild.queue[0];
            return `<div class="air"><span class="badge is-air">À l’antenne</span>
              <div><div class="title">${esc(job.title)}</div><div class="soft"><a href="#/guilds/${guild.id}/queue">${esc(guildName(guild))}</a> · ${esc(job.by)} · ${plural(guild.queue.length - 1, 'média en attente', 'médias en attente')}</div>
              <div class="progress"><span style="width:${job.progress}%"></span></div></div>
              <button class="btn is-small" data-act="stop" data-guild="${guild.id}">${ic('stop')}Arrêter</button></div>`;
          })
          .join('')
      : '<p class="empty">Rien à l’antenne pour le moment.</p>';

    const html = `
      <div class="page-head"><div><h1>Tableau de bord</h1><p class="sub">État de l’instance, mis à jour en direct.</p></div>
        <div class="actions"><a class="btn" href="${BOT_INVITE_URL}" target="_blank" rel="noopener">${ic('external')}Inviter le bot sur Discord</a>
        <button class="btn is-primary" data-act="create-guild">${ic('plus')}Créer une guilde</button></div></div>

      <div class="stack">
        <section class="health" aria-label="Santé du serveur">
          ${[
            ['Serveur', `v${server.version} · ${server.uptime}`, 'ok'],
            ['Base SQLite', '48 Mo · WAL', 'ok'],
            ['Bot Discord', '4 serveurs', 'ok'],
            ['WebSocket', `${connected} overlays`, 'ok'],
            ['Ingestion', '1 en cours / 3 max', 'ok'],
            ['ffmpeg · yt-dlp', '7.1 · 2026.11.04', 'ok'],
            ['Synthèse vocale', 'Piper fr, en', 'ok'],
          ]
            .map(([key, value, status]) => `<div><div class="k"><span class="dot is-${status}"></span>${key}</div><div class="v">${value}</div></div>`)
            .join('')}
        </section>

        ${outdated ? `<div class="notice">${ic('info')}<div><strong>${plural(outdated, 'overlay est', 'overlays sont')} sous la version minimale ${MIN_OVERLAY_VERSION}.</strong><div class="soft">Ils seront refusés à leur prochaine connexion tant qu’ils ne se mettent pas à jour.</div></div><a class="btn is-small" href="#/devices" data-act="filter-outdated">Voir les overlays</a></div>` : ''}
        ${failed ? `<div class="notice is-err">${ic('alert')}<div><strong>${plural(failed, 'ingestion a échoué', 'ingestions ont échoué')} ces dernières 24 h.</strong><div class="soft">Dont une demande de cookie TikTok.</div></div><a class="btn is-small" href="#/media">Voir les ingestions</a></div>` : ''}

        <section class="grid is-tiles" aria-label="Chiffres clés">
          <div class="card tile"><div class="label">${ic('guilds')}Guildes</div><div class="value">${guilds.length}</div><div class="foot">${guilds.filter((g) => g.kind === 'native').length} Discord · ${guilds.filter((g) => g.kind !== 'native').length} indépendantes</div></div>
          <div class="card tile"><div class="label">${ic('device')}Overlays en ligne</div><div class="value">${connected} <small>/ ${devices.length}</small></div><div class="foot">${guilds.reduce((s, g) => s + g.members.length, 0)} membres au total</div></div>
          <div class="card tile"><div class="label">${ic('key')}Codes actifs</div><div class="value">${activeCodes}</div><div class="foot">invitations et appairages</div></div>
          <div class="card tile"><div class="label">${ic('plug')}Clients d’ingestion</div><div class="value">${guilds.reduce((s, g) => s + g.ingest.length, 0)}</div><div class="foot">extensions navigateur</div></div>
          <div class="card tile"><div class="label">${ic('play')}Lectures (24 h)</div><div class="value">342</div><div class="foot">7 en échec</div></div>
        </section>

        <div class="grid is-2">
          <section class="card"><div class="card-head"><h2>À l’antenne</h2><span class="meta">${plural(onAir.length, 'guilde', 'guildes')}</span></div>${air}</section>
          <section class="card"><div class="card-head"><h2>Stockage</h2><span class="spacer"></span><a href="#/media">Détail</a></div>
            <div class="card-body stack">
              <div class="meter-row"><span>Cache média <span class="faint">(médias hors board)</span></span><span class="num soft">${bytes(server.cacheUsed)} / ${bytes(CACHE_MAX)}</span>${meter(server.cacheUsed, CACHE_MAX, 'Cache média')}</div>
              <div class="meter-row"><span>Boards, toutes guildes</span><span class="num soft">${bytes(boardUsed)}</span></div>
              ${fullest.map((guild) => `<div class="meter-row"><a href="#/guilds/${guild.id}/board">${esc(guildName(guild))}</a><span class="num soft">${bytes(guild.board.usedBytes)} / ${bytes(BOARD_MAX)}</span>${meter(guild.board.usedBytes, BOARD_MAX, guildName(guild))}</div>`).join('')}
            </div></section>
        </div>

        <section class="card"><div class="card-head"><h2>Activité récente</h2><span class="spacer"></span><a href="#/activity">Tout le journal</a></div>
          <div class="table-wrap">${activityTable(activity.slice(0, 5))}</div></section>
      </div>`;
    return { nav: 'dashboard', title: 'Tableau de bord', crumbs: crumb(['Tableau de bord']), html };
  }

  function activityTable(rows) {
    return `<table class="tbl"><thead><tr><th>Quand</th><th>Qui</th><th>Action</th><th>Guilde</th><th>Route</th><th class="is-num">Statut</th></tr></thead><tbody>
      ${rows
        .map(
          (row) => `<tr><td class="soft">${ago(row.min)}</td><td>${esc(row.actor)}</td><td class="primary">${esc(row.action)}</td>
          <td>${row.guild ? `<a href="#/guilds/${row.guild.id}">${esc(guildName(row.guild))}</a>` : '<span class="faint">—</span>'}</td>
          <td class="mono soft">${esc(row.route)}</td><td class="is-num"><span class="badge ${row.status < 300 ? 'is-ok' : 'is-err'}">${row.status}</span></td></tr>`,
        )
        .join('')}</tbody></table>`;
  }

  // ----------------------------------------------------------------- guilds
  function kindBadge(guild) {
    const [label, cls] = KIND_BADGE[guild.kind];
    return `<span class="badge ${cls}">${label}</span>`;
  }

  function guildsPage() {
    regionRenderer = () => {
      const query = state.filters.guildQuery.trim().toLowerCase();
      const rows = guilds.filter((guild) => {
        const kindOk = state.filters.guildKind === 'all' || (state.filters.guildKind === 'air' ? guild.queue.length > 0 : state.filters.guildKind === 'discord' ? guild.kind === 'native' : guild.kind !== 'native');
        return kindOk && (!query || guildName(guild).toLowerCase().includes(query) || guild.id.includes(query));
      });
      if (!rows.length) {
        return '<div class="table-wrap"><p class="empty">Aucune guilde ne correspond à cette recherche.</p></div>';
      }
      return `<div class="table-wrap"><table class="tbl"><thead><tr><th>Guilde</th><th>Type</th><th class="is-num">Membres</th><th class="is-num">Overlays</th><th>Board</th><th>Lecture</th><th>Dernière activité</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
        ${rows
          .map((guild) => {
            const online = guild.devices.filter((device) => device.connected).length;
            return `<tr class="${guild.queue.length ? 'is-air' : ''}">
              <td><a class="primary" href="#/guilds/${guild.id}">${esc(guildName(guild))}</a><span class="under mono">${guild.id}</span></td>
              <td>${kindBadge(guild)}${guild.kind === 'native' && !guild.botPresent ? ' <span class="badge is-err">Bot absent</span>' : ''}</td>
              <td class="is-num">${guild.members.length}</td>
              <td class="is-num"><span class="dot ${online ? 'is-ok' : ''}" style="display:inline-block;margin-right:6px"></span>${online} / ${guild.devices.length}</td>
              <td><span class="mini-meter">${meter(guild.board.usedBytes, BOARD_MAX, 'Board')}</span><span class="num soft">${bytes(guild.board.usedBytes)}</span></td>
              <td>${guild.queue.length ? `<span class="badge is-air">À l’antenne</span> <span class="soft">+${guild.queue.length - 1}</span>` : '<span class="faint">Au repos</span>'}</td>
              <td class="soft">${ago(guild.lastActivity)}</td>
              <td class="is-actions"><a class="btn is-small" href="#/guilds/${guild.id}">Ouvrir</a></td></tr>`;
          })
          .join('')}</tbody></table></div>
        <p class="table-foot">${plural(rows.length, 'guilde', 'guildes')} sur ${guilds.length}</p>`;
    };
    const kinds = [['all', 'Toutes'], ['discord', 'Discord'], ['standalone', 'Indépendantes'], ['air', 'À l’antenne']];
    const html = `
      <div class="page-head"><div><h1>Guildes</h1><p class="sub">Serveurs Discord où le bot est installé et groupes indépendants.</p></div>
        <div class="actions"><a class="btn" href="${BOT_INVITE_URL}" target="_blank" rel="noopener">${ic('external')}Inviter le bot sur Discord</a>
        <button class="btn is-primary" data-act="create-guild">${ic('plus')}Créer une guilde</button></div></div>
      <div class="toolbar">
        <div class="search">${ic('search')}<input class="field" type="search" placeholder="Nom ou identifiant" aria-label="Rechercher une guilde" data-filter="guildQuery" value="${esc(state.filters.guildQuery)}"></div>
        <div class="seg" role="group" aria-label="Filtrer par type">${kinds.map(([id, label]) => `<button type="button" data-seg="guildKind" data-value="${id}" aria-pressed="${state.filters.guildKind === id}">${label}</button>`).join('')}</div>
      </div>
      <div id="region"></div>`;
    return { nav: 'guilds', title: 'Guildes', crumbs: crumb(['Guildes']), html };
  }

  // ----------------------------------------------------------- guild detail
  function guildDetail(id, tab) {
    const guild = findGuild(id);
    if (!guild) {
      return { nav: 'guilds', title: 'Guilde introuvable', crumbs: crumb(['Guildes', '#/guilds'], ['Introuvable']), html: '<div class="card"><p class="empty">Cette guilde n’existe plus. <a href="#/guilds">Retour à la liste</a></p></div>' };
    }
    const tabs = [
      ['overview', 'Vue d’ensemble'],
      ['members', 'Membres', guild.members.length],
      ['devices', 'Overlays', guild.devices.length],
      ['access', 'Invitations et codes', guild.invites.filter((i) => i.status === 'active').length + guild.pairing.filter((i) => i.status === 'active').length],
      ['board', 'Board média', guild.board.items],
      ['queue', 'File d’attente', guild.queue.length],
      ['settings', 'Réglages'],
      ['discord', 'Discord'],
      ['danger', 'Zone de danger'],
    ];
    const views = { overview: guildOverview, members: guildMembers, devices: guildDevices, access: guildAccess, board: guildBoard, queue: guildQueue, settings: guildSettings, discord: guildDiscord, danger: guildDanger };
    const current = views[tab] ? tab : 'overview';
    const html = `
      <div class="page-head"><div><h1>${esc(guildName(guild))}</h1>
        <div class="guild-id">${kindBadge(guild)}${guild.queue.length ? '<span class="badge is-air">À l’antenne</span>' : ''}
          <span class="mono soft">${guild.id}</span><button class="btn is-quiet is-icon" data-act="copy" data-text="${guild.id}" aria-label="Copier l’identifiant">${ic('copy')}</button></div></div>
        <div class="actions">
          <button class="btn" data-act="admin-code" data-guild="${guild.id}">${ic('key')}Code admin</button>
          <button class="btn" data-act="stop" data-guild="${guild.id}"${guild.queue.length ? '' : ' disabled'}>${ic('stop')}Arrêter la lecture</button></div></div>
      <nav class="tabs" aria-label="Sections de la guilde">${tabs
        .map(([key, label, count]) => `<a href="#/guilds/${guild.id}/${key}" class="${key === 'danger' ? 'is-danger' : ''}"${key === current ? ' aria-current="page"' : ''}>${label}${count != null ? `<span class="count">${count}</span>` : ''}</a>`)
        .join('')}</nav>
      ${views[current](guild)}`;
    return { nav: 'guilds', title: guildName(guild), crumbs: crumb(['Guildes', '#/guilds'], [guildName(guild)]), html };
  }

  function guildOverview(guild) {
    const online = guild.devices.filter((device) => device.connected).length;
    const job = guild.queue[0];
    const recent = activity.filter((row) => row.guild === guild).slice(0, 4);
    return `<div class="stack">
      <div class="grid is-tiles">
        <div class="card tile"><div class="label">${ic('guilds')}Membres</div><div class="value">${guild.members.length}</div><div class="foot">${guild.members.filter((m) => m.role === 'admin').length} admin · ${guild.members.filter((m) => m.role === 'guest').length} invités</div></div>
        <div class="card tile"><div class="label">${ic('device')}Overlays en ligne</div><div class="value">${online} <small>/ ${guild.devices.length}</small></div><div class="foot"><a href="#/guilds/${guild.id}/devices">Voir les appareils</a></div></div>
        <div class="card tile"><div class="label">${ic('disk')}Board média</div><div class="value">${guild.board.items} <small>médias</small></div><div class="foot" style="margin-top:8px">${meter(guild.board.usedBytes, BOARD_MAX, 'Board')}<span class="num">${bytes(guild.board.usedBytes)} / ${bytes(BOARD_MAX)}</span></div></div>
        <div class="card tile"><div class="label">${ic('play')}Lectures</div><div class="value">${guild.playback.done}</div><div class="foot">${guild.playback.pending} en attente · ${guild.playback.failed} en échec</div></div>
      </div>
      <div class="grid is-2">
        <section class="card"><div class="card-head"><h2>À l’antenne</h2><span class="spacer"></span><a href="#/guilds/${guild.id}/queue">File d’attente</a></div>
          ${job ? `<div class="air"><span class="badge is-air">En lecture</span><div><div class="title">${esc(job.title)}</div><div class="soft">${KIND_LABEL[job.kind]} · ${job.duration} s · ${esc(job.by)} · ${esc(job.source)}</div><div class="progress"><span style="width:${job.progress}%"></span></div></div><button class="btn is-small" data-act="stop" data-guild="${guild.id}">${ic('stop')}Arrêter</button></div>` : '<p class="empty">Rien à l’antenne.</p>'}
        </section>
        <section class="card"><div class="card-head"><h2>Fiche</h2></div><div class="card-body"><dl class="kv">
          <dt>Type</dt><dd>${KIND_BADGE[guild.kind][0]}</dd>
          <dt>Serveur Discord</dt><dd>${guild.discordServerId ? `<span class="mono">${guild.discordServerId}</span>` : '<span class="faint">Aucun</span>'}</dd>
          <dt>Créée</dt><dd>${ago(guild.createdDays * 1440)}</dd>
          <dt>Dernière activité</dt><dd>${ago(guild.lastActivity)}</dd>
          <dt>Durée par défaut</dt><dd>${guild.settings.defaultMediaTime ? `${guild.settings.defaultMediaTime} s` : '<span class="soft">Valeur du serveur (5 s)</span>'}</dd>
          <dt>Clients d’ingestion</dt><dd>${guild.ingest.length}</dd>
        </dl></div></section>
      </div>
      <section class="card"><div class="card-head"><h2>Dernières actions d’administration</h2></div>
        ${recent.length ? `<div class="table-wrap">${activityTable(recent)}</div>` : '<p class="empty">Aucune action récente sur cette guilde.</p>'}</section>
    </div>`;
  }

  function roleSelect(guild, member) {
    return `<select class="field is-inline" aria-label="Rôle de ${esc(member.name)}" data-change="role" data-guild="${guild.id}" data-id="${member.id}">
      ${Object.entries(ROLE_LABEL).map(([value, label]) => `<option value="${value}"${member.role === value ? ' selected' : ''}>${label}</option>`).join('')}</select>`;
  }

  function guildMembers(guild) {
    return `<div class="toolbar"><p class="soft">Le rôle fixe les droits de tous les appareils du membre.</p><span class="spacer"></span>
        <button class="btn" data-act="create-invite" data-guild="${guild.id}">${ic('plus')}Inviter un membre</button></div>
      <div class="table-wrap"><table class="tbl"><thead><tr><th>Membre</th><th>Compte Discord</th><th>Rôle</th><th class="is-num">Appareils</th><th>Présence</th><th>Arrivée</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
      ${guild.members
        .map((member) => {
          const devices = guild.devices.filter((device) => device.memberId === member.id);
          const online = devices.some((device) => device.connected);
          return `<tr><td class="primary">${who(member.name)}</td>
            <td>${member.discordUserId ? `<span class="mono soft">${member.discordUserId}</span>` : '<span class="faint">Non lié</span>'}</td>
            <td>${roleSelect(guild, member)}</td>
            <td class="is-num">${devices.length}</td>
            <td>${online ? '<span class="badge is-ok"><span class="dot is-ok"></span>En ligne</span>' : `<span class="soft">${devices.length ? ago(member.lastSeen) : 'Aucun appareil'}</span>`}</td>
            <td class="soft">${ago(member.joinedDays * 1440)}</td>
            <td class="is-actions"><button class="btn is-small is-quiet is-danger" data-act="kick" data-guild="${guild.id}" data-id="${member.id}">Exclure</button></td></tr>`;
        })
        .join('')}</tbody></table></div>`;
  }

  function deviceRow(guild, device, withGuild) {
    const member = memberOf(guild, device.memberId);
    const old = versionBelow(device.version, MIN_OVERLAY_VERSION);
    const presence = device.connected
      ? '<span class="badge is-ok"><span class="dot is-ok"></span>En ligne</span>'
      : device.muted
        ? '<span class="badge">En sourdine</span>'
        : `<span class="soft">${ago(device.lastSeen)}</span>`;
    return `<tr class="${device.connected ? '' : 'is-off'}">
      <td><span class="primary">${esc(device.label)}</span><span class="under mono">${device.id.slice(0, 13)}…</span></td>
      ${withGuild ? `<td><a href="#/guilds/${guild.id}/devices">${esc(guildName(guild))}</a></td>` : ''}
      <td>${member ? who(member.name) : '—'}</td>
      <td><span class="badge ${member && member.role === 'admin' ? 'is-admin' : ''}">${member ? ROLE_LABEL[member.role] : '—'}</span></td>
      <td><span class="badge ${device.sessionMode === 'normal' ? 'is-info' : 'is-outline'}">${MODE_LABEL[device.sessionMode]}</span></td>
      <td>${presence}</td>
      <td><span class="mono">${device.version}</span>${old ? ' <span class="badge is-err">Obsolète</span>' : ''}<span class="under">${device.os}</span></td>
      <td class="is-actions"><button class="btn is-small" data-act="device" data-guild="${guild.id}" data-id="${device.id}">${ic('shield')}Droits</button>
        <button class="btn is-small is-quiet is-danger" data-act="revoke-device" data-guild="${guild.id}" data-id="${device.id}">Révoquer</button></td></tr>`;
  }

  const deviceHead = (withGuild) => `<thead><tr><th>Appareil</th>${withGuild ? '<th>Guilde</th>' : ''}<th>Membre</th><th>Rôle</th><th>Session</th><th>État</th><th>Version</th><th><span class="sr-only">Actions</span></th></tr></thead>`;

  function guildDevices(guild) {
    return `<div class="toolbar"><p class="soft">Chaque overlay appairé à cette guilde. « Droits » règle le mode de session de l’appareil.</p><span class="spacer"></span>
        <button class="btn" data-act="device-code" data-guild="${guild.id}">${ic('plus')}Code d’appairage</button></div>
      ${guild.devices.length ? `<div class="table-wrap"><table class="tbl">${deviceHead(false)}<tbody>${guild.devices.map((device) => deviceRow(guild, device, false)).join('')}</tbody></table></div>` : '<div class="card"><p class="empty">Aucun overlay appairé.</p></div>'}`;
  }

  const INVITE_STATUS = { active: ['Active', 'is-ok'], expired: ['Expirée', ''], revoked: ['Révoquée', 'is-err'], used: ['Épuisée', ''] };
  const CODE_STATUS = { active: ['Actif', 'is-ok'], expired: ['Expiré', ''], used: ['Utilisé', ''] };

  function inviteRows(guild, withGuild) {
    return guild.invites
      .map((invite) => {
        const [label, cls] = INVITE_STATUS[invite.status];
        const active = invite.status === 'active';
        return `<tr class="${active ? '' : 'is-off'}"><td><span class="code">${invite.code.slice(0, 4)}-${invite.code.slice(4)}</span></td>
          ${withGuild ? `<td><a href="#/guilds/${guild.id}/access">${esc(guildName(guild))}</a></td>` : ''}
          <td><span class="badge">Invitation</span></td>
          <td><span class="badge ${invite.role === 'admin' ? 'is-admin' : ''}">${ROLE_LABEL[invite.role]}</span></td>
          <td><span class="badge ${cls}">${label}</span></td>
          <td class="soft">${active ? (invite.expiresHours ? `dans ${invite.expiresHours >= 48 ? `${Math.round(invite.expiresHours / 24)} j` : `${invite.expiresHours} h`}` : 'jamais') : '—'}</td>
          <td class="is-num">${invite.useCount}${invite.maxUses ? ` / ${invite.maxUses}` : ''}</td>
          <td class="soft">${esc(invite.createdBy)}, ${ago(invite.createdMin)}</td>
          <td class="is-actions">${active ? `<button class="btn is-small is-quiet is-icon" data-act="copy" data-text="${invite.code}" aria-label="Copier le code ${invite.code}">${ic('copy')}</button><button class="btn is-small is-quiet is-danger" data-act="revoke-invite" data-guild="${guild.id}" data-id="${invite.code}">Révoquer</button>` : ''}</td></tr>`;
      })
      .join('');
  }

  function pairingRows(guild, withGuild) {
    return guild.pairing
      .map((item) => {
        const [label, cls] = CODE_STATUS[item.status];
        const active = item.status === 'active';
        return `<tr class="${active ? '' : 'is-off'}"><td><span class="code">${item.code}</span></td>
          ${withGuild ? `<td><a href="#/guilds/${guild.id}/access">${esc(guildName(guild))}</a></td>` : ''}
          <td><span class="badge">Appairage Discord</span></td>
          <td><span class="badge">${item.mode === 'normal' ? 'Membre' : 'Invité'}</span></td>
          <td><span class="badge ${cls}">${label}</span></td>
          <td class="soft">${active ? `dans ${item.expiresMin} min` : '—'}</td>
          <td class="is-num">${item.status === 'used' ? '1 / 1' : '0 / 1'}</td>
          <td class="soft">${esc(item.authorName)}, ${ago(item.createdMin)}</td>
          <td class="is-actions">${active ? `<button class="btn is-small is-quiet is-danger" data-act="revoke-code" data-guild="${guild.id}" data-id="${item.code}">Révoquer</button>` : ''}</td></tr>`;
      })
      .join('');
  }

  const codeHead = (withGuild) => `<thead><tr><th>Code</th>${withGuild ? '<th>Guilde</th>' : ''}<th>Nature</th><th>Accorde</th><th>État</th><th>Expire</th><th class="is-num">Utilisations</th><th>Créé par</th><th><span class="sr-only">Actions</span></th></tr></thead>`;

  function guildAccess(guild) {
    const rows = inviteRows(guild, false) + pairingRows(guild, false);
    return `<div class="toolbar"><p class="soft">Invitations créées dans l’overlay et codes d’appairage émis depuis Discord (<span class="mono">/overlay-code</span>).</p><span class="spacer"></span>
        <button class="btn" data-act="purge-codes" data-guild="${guild.id}">${ic('trash')}Purger les expirés</button>
        <button class="btn is-primary" data-act="create-invite" data-guild="${guild.id}">${ic('plus')}Créer une invitation</button></div>
      ${rows ? `<div class="table-wrap"><table class="tbl">${codeHead(false)}<tbody>${rows}</tbody></table></div>` : '<div class="card"><p class="empty">Aucun code pour cette guilde.</p></div>'}`;
  }

  function guildBoard(guild) {
    const board = guild.board;
    return `<div class="stack"><div class="grid is-2">
      <section class="card"><div class="card-head"><h2>Occupation</h2></div><div class="card-body stack">
        <div class="meter-row"><span>${board.items} médias</span><span class="num soft">${bytes(board.usedBytes)} / ${bytes(BOARD_MAX)}</span>${meter(board.usedBytes, BOARD_MAX, 'Board')}</div>
        <dl class="kv"><dt>Vidéos</dt><dd class="num">${board.kinds.video}</dd><dt>Sons</dt><dd class="num">${board.kinds.audio}</dd><dt>Images</dt><dd class="num">${board.kinds.image}</dd><dt>Favoris</dt><dd class="num">${board.favorites}</dd></dl>
      </div></section>
      <section class="card"><div class="card-head"><h2>Quota de cette guilde</h2></div><div class="card-body stack">
        <p class="soft">Par défaut, la limite du serveur s’applique (<span class="mono">MEDIA_BOARD_MAX_TOTAL_MB</span>, 15 Go).</p>
        <div class="row is-wrap"><label for="quota">Quota propre (Go)</label><input class="field is-inline num" id="quota" type="number" min="1" max="200" placeholder="15" style="width:90px"><button class="btn" data-act="toast" data-text="Quota de la guilde enregistré.">Appliquer</button></div>
        <div class="row is-wrap"><button class="btn is-danger" data-act="clear-board" data-guild="${guild.id}">${ic('trash')}Vider le board</button></div>
      </div></section></div>
      <section class="card"><div class="card-head"><h2>Derniers ajouts</h2><span class="meta">aperçu</span></div>
        ${board.recent.length ? `<div class="table-wrap"><table class="tbl"><thead><tr><th>Titre</th><th>Type</th><th class="is-num">Durée</th><th class="is-num">Taille</th><th class="is-num">Lectures</th><th>Ajouté par</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
          ${board.recent.map((item, index) => `<tr><td class="primary">${esc(item.title)}</td><td><span class="badge">${KIND_LABEL[item.kind]}</span></td><td class="is-num">${item.duration} s</td><td class="is-num">${bytes(item.size)}</td><td class="is-num">${item.plays}</td><td class="soft">${esc(item.by)}, ${ago(item.addedMin)}</td>
            <td class="is-actions"><button class="btn is-small is-quiet is-danger" data-act="delete-media" data-guild="${guild.id}" data-id="${index}">Supprimer</button></td></tr>`).join('')}</tbody></table></div>` : '<p class="empty">Board vide.</p>'}
      </section></div>`;
  }

  function guildQueue(guild) {
    return `<div class="toolbar"><p class="soft">${guild.queue.length ? `${plural(guild.queue.length, 'média', 'médias')} dans la file, dont celui à l’antenne.` : 'La file est vide.'}</p><span class="spacer"></span>
        <button class="btn" data-act="clear-queue" data-guild="${guild.id}"${guild.queue.length > 1 ? '' : ' disabled'}>Vider la file</button>
        <button class="btn" data-act="stop" data-guild="${guild.id}"${guild.queue.length ? '' : ' disabled'}>${ic('stop')}Arrêter la lecture</button></div>
      ${guild.queue.length ? `<div class="table-wrap"><table class="tbl"><thead><tr><th class="is-num">#</th><th>Média</th><th>Type</th><th class="is-num">Durée</th><th>Demandé par</th><th>Origine</th><th>État</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
        ${guild.queue.map((job, index) => `<tr class="${job.status === 'playing' ? 'is-air' : ''}"><td class="is-num soft">${index + 1}</td><td class="primary">${esc(job.title)}</td><td><span class="badge">${KIND_LABEL[job.kind]}</span></td><td class="is-num">${job.duration} s</td><td>${who(job.by)}</td><td class="soft">${esc(job.source)}</td>
          <td>${job.status === 'playing' ? '<span class="badge is-air">À l’antenne</span>' : '<span class="badge">En attente</span>'}</td>
          <td class="is-actions">${job.status === 'playing' ? '' : `<button class="btn is-small is-quiet is-danger" data-act="remove-job" data-guild="${guild.id}" data-id="${job.id}">Retirer</button>`}</td></tr>`).join('')}</tbody></table></div>` : '<div class="card"><p class="empty">Aucune lecture en cours ni en attente.</p></div>'}
      <p class="table-foot">Historique : ${guild.playback.done} lectures terminées, ${guild.playback.failed} en échec (conservées 24 h).</p>`;
  }

  function guildSettings(guild) {
    const s = guild.settings;
    return `<section class="card"><div class="card-head"><h2>Réglages de la guilde</h2><span class="meta">pour tous ses appareils</span></div><div class="card-body">
      <form class="form" data-form="guild-settings" data-guild="${guild.id}">
        <label for="gs-name">Nom de la guilde</label><div><input class="field" id="gs-name" maxlength="60" value="${esc(guild.name)}" placeholder="Guild ${guild.id}"></div>
        <label for="gs-default">Durée par défaut (s)</label><div><input class="field" id="gs-default" type="number" min="0" max="86400" value="${s.defaultMediaTime ?? ''}" placeholder="Valeur du serveur (5)"><p class="hint">Durée d’affichage d’une image ou d’un texte sans durée propre.</p></div>
        <label for="gs-max">Durée maximale (s)</label><div><input class="field" id="gs-max" type="number" min="0" max="86400" value="${s.maxMediaTime ?? ''}" placeholder="Sans limite"></div>
        <span class="lbl">Affichage</span><div><label class="switch"><input type="checkbox" id="gs-full"${s.displayMediaFull ? ' checked' : ''}>Médias en plein écran</label></div>
        <span class="lbl">Invités</span><div><label class="switch"><input type="checkbox" id="gs-guests"${s.guestsCanViewBoard ? ' checked' : ''}>Peuvent consulter le board</label></div>
        <span></span><div class="row"><button class="btn is-primary" type="submit">Enregistrer les réglages</button></div>
      </form></div></section>`;
  }

  function guildDiscord(guild) {
    let body;
    if (guild.kind === 'native') {
      body = `<dl class="kv"><dt>Rattachement</dt><dd>Cette guilde <strong>est</strong> un serveur Discord.</dd><dt>Serveur</dt><dd class="mono">${guild.discordServerId}</dd>
        <dt>Bot</dt><dd>${guild.botPresent ? '<span class="badge is-ok">Présent</span>' : '<span class="badge is-err">Absent du serveur</span>'}</dd><dt>Commandes</dt><dd><span class="mono">/send</span>, <span class="mono">/talk</span>, <span class="mono">/stop</span>, <span class="mono">/overlay-code</span> · synchronisées</dd></dl>
        <div class="row is-wrap" style="margin-top:14px">${guild.botPresent ? `<button class="btn is-danger" data-act="leave-discord" data-guild="${guild.id}">Faire quitter le bot</button>` : `<a class="btn" href="${BOT_INVITE_URL}&guild_id=${guild.id}" target="_blank" rel="noopener">${ic('external')}Réinviter le bot</a>`}</div>`;
    } else if (guild.kind === 'linked') {
      body = `<dl class="kv"><dt>Rattachement</dt><dd>Guilde indépendante liée à un serveur Discord.</dd><dt>Serveur</dt><dd>${esc(guild.discordName || '')} <span class="mono soft">${guild.discordServerId}</span></dd><dt>Bot</dt><dd><span class="badge is-ok">Présent</span></dd></dl>
        <div class="row is-wrap" style="margin-top:14px"><button class="btn is-danger" data-act="unlink-discord" data-guild="${guild.id}">Délier le serveur Discord</button></div>`;
    } else {
      body = `<p class="soft">Cette guilde indépendante n’est liée à aucun serveur Discord. Pour la lier : inviter le bot sur le serveur, générer un code, puis taper la commande dans Discord.</p>
        <div class="row is-wrap" style="margin-top:14px"><a class="btn" href="${BOT_INVITE_URL}" target="_blank" rel="noopener">${ic('external')}Inviter le bot</a>
        <button class="btn is-primary" data-act="link-code" data-guild="${guild.id}">Générer un code de liaison</button></div>`;
    }
    return `<section class="card"><div class="card-head"><h2>Liaison Discord</h2></div><div class="card-body">${body}</div></section>`;
  }

  function guildDanger(guild) {
    const rows = [
      ['Révoquer tous les accès', 'Arrête la lecture, révoque overlays et clients d’ingestion, retire les membres, supprime codes et invitations. La guilde, son board et ses réglages sont conservés.', 'Révoquer les accès', 'revoke-all'],
      ['Vider le board média', `Supprime les ${guild.board.items} médias du board et les fichiers devenus orphelins (${bytes(guild.board.usedBytes)}).`, 'Vider le board', 'clear-board'],
    ];
    if (guild.kind === 'linked') {
      rows.push(['Délier le serveur Discord', 'Les commandes de ce serveur Discord ne viseront plus cette guilde. Aucune donnée n’est supprimée.', 'Délier', 'unlink-discord']);
    }
    if (guild.kind !== 'none' && guild.botPresent) {
      rows.push(['Faire quitter le bot', 'Le bot quitte le serveur Discord. Les données de la guilde restent sur le serveur LiveChat.', 'Faire quitter le bot', 'leave-discord']);
    }
    rows.push(['Supprimer la guilde', 'Supprime définitivement membres, overlays, codes, file, board et réglages. Irréversible.', 'Supprimer la guilde', 'delete-guild']);
    return `<section class="card is-danger"><div class="card-head"><h2>Zone de danger</h2><span class="meta">chaque action demande une confirmation</span></div>
      ${rows.map(([title, text, label, act]) => `<div class="danger-row"><div><h3>${title}</h3><p>${text}</p></div><button class="btn ${act === 'delete-guild' ? 'is-danger-solid' : 'is-danger'}" data-act="${act}" data-guild="${guild.id}">${label}</button></div>`).join('')}
    </section>`;
  }

  // ------------------------------------------------------- global: overlays
  function devicesPage() {
    regionRenderer = () => {
      const query = state.filters.deviceQuery.trim().toLowerCase();
      const rows = allDevices().filter(({ device, guild }) => {
        const member = memberOf(guild, device.memberId);
        const stateOk =
          state.filters.deviceState === 'all' ||
          (state.filters.deviceState === 'online' && device.connected) ||
          (state.filters.deviceState === 'readonly' && device.sessionMode === 'invite_read_only') ||
          (state.filters.deviceState === 'outdated' && versionBelow(device.version, MIN_OVERLAY_VERSION));
        return stateOk && (!query || `${device.label} ${guildName(guild)} ${member ? member.name : ''}`.toLowerCase().includes(query));
      });
      if (!rows.length) {
        return '<div class="table-wrap"><p class="empty">Aucun overlay ne correspond.</p></div>';
      }
      return `<div class="table-wrap"><table class="tbl">${deviceHead(true)}<tbody>${rows.map(({ device, guild }) => deviceRow(guild, device, true)).join('')}</tbody></table></div><p class="table-foot">${plural(rows.length, 'overlay', 'overlays')} sur ${allDevices().length}</p>`;
    };
    const states = [['all', 'Tous'], ['online', 'En ligne'], ['readonly', 'Lecture seule'], ['outdated', 'Obsolètes']];
    const html = `<div class="page-head"><div><h1>Overlays</h1><p class="sub">Tous les appareils appairés, toutes guildes confondues. Version minimale acceptée : <span class="mono">${MIN_OVERLAY_VERSION}</span>.</p></div></div>
      <div class="toolbar"><div class="search">${ic('search')}<input class="field" type="search" placeholder="Appareil, membre ou guilde" aria-label="Rechercher un overlay" data-filter="deviceQuery" value="${esc(state.filters.deviceQuery)}"></div>
        <div class="seg" role="group" aria-label="Filtrer par état">${states.map(([id, label]) => `<button type="button" data-seg="deviceState" data-value="${id}" aria-pressed="${state.filters.deviceState === id}">${label}</button>`).join('')}</div></div>
      <div id="region"></div>`;
    return { nav: 'devices', title: 'Overlays', crumbs: crumb(['Overlays']), html };
  }

  // ---------------------------------------------------------- global: codes
  function codesPage() {
    regionRenderer = () => {
      const status = state.filters.codeStatus;
      const keep = (item) => status === 'all' || (status === 'active' ? item.status === 'active' : item.status !== 'active');
      const rows = guilds
        .map((guild) => {
          const filtered = { ...guild, invites: guild.invites.filter(keep), pairing: guild.pairing.filter(keep) };
          return inviteRows(filtered, true) + pairingRows(filtered, true);
        })
        .join('');
      return rows ? `<div class="table-wrap"><table class="tbl">${codeHead(true)}<tbody>${rows}</tbody></table></div>` : '<div class="table-wrap"><p class="empty">Aucun code.</p></div>';
    };
    const statuses = [['all', 'Tous'], ['active', 'Actifs'], ['expired', 'Expirés ou utilisés']];
    const html = `<div class="page-head"><div><h1>Codes et invitations</h1><p class="sub">Tout ce qui permet d’entrer dans une guilde : invitations d’overlay et codes d’appairage émis depuis Discord.</p></div>
        <div class="actions"><button class="btn" data-act="purge-codes">${ic('trash')}Purger les expirés</button></div></div>
      <div class="toolbar"><div class="seg" role="group" aria-label="Filtrer par état">${statuses.map(([id, label]) => `<button type="button" data-seg="codeStatus" data-value="${id}" aria-pressed="${state.filters.codeStatus === id}">${label}</button>`).join('')}</div></div>
      <div id="region"></div>`;
    return { nav: 'codes', title: 'Codes et invitations', crumbs: crumb(['Codes et invitations']), html };
  }

  // --------------------------------------------------------- global: ingest
  function ingestPage() {
    const rows = guilds.flatMap((guild) => guild.ingest.map((client) => ({ guild, client })));
    const html = `<div class="page-head"><div><h1>Clients d’ingestion</h1><p class="sub">Extensions de navigateur et scripts autorisés à envoyer des médias à une guilde (<span class="mono">POST /ingest</span>).</p></div>
        <div class="actions"><button class="btn is-primary" data-act="create-ingest">${ic('plus')}Créer un client</button></div></div>
      <div class="table-wrap"><table class="tbl"><thead><tr><th>Client</th><th>Guilde</th><th>Auteur par défaut</th><th>Dernière utilisation</th><th>Créé</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
        ${rows.map(({ guild, client }) => `<tr><td><span class="primary">${esc(client.label)}</span><span class="under mono">${client.id.slice(0, 13)}…</span></td><td><a href="#/guilds/${guild.id}">${esc(guildName(guild))}</a></td><td>${who(client.authorName)}</td>
          <td class="soft">${ago(client.lastSeen)}</td><td class="soft">${ago(client.createdDays * 1440)}</td>
          <td class="is-actions"><button class="btn is-small is-quiet is-danger" data-act="revoke-ingest" data-guild="${guild.id}" data-id="${client.id}">Révoquer</button></td></tr>`).join('')}</tbody></table></div>
      <p class="table-foot">Le jeton d’un client n’est affiché qu’une fois, à sa création. Depuis l’overlay, un membre peut aussi appairer son extension avec un code (<span class="mono">POST /overlay/ingest-code</span>).</p>`;
    return { nav: 'ingest', title: 'Clients d’ingestion', crumbs: crumb(['Clients d’ingestion']), html };
  }

  // ---------------------------------------------------------- global: media
  function mediaPage() {
    const INGEST = { processing: ['En cours', 'is-info'], ready: ['Prêt', 'is-ok'], failed: ['Échec', 'is-err'] };
    const total = guilds.reduce((sum, guild) => sum + guild.board.usedBytes, 0);
    const html = `<div class="page-head"><div><h1>Médias et stockage</h1><p class="sub">Cache de lecture, boards par guilde et ingestions récentes.</p></div>
        <div class="actions"><button class="btn is-danger" data-act="purge-cache">${ic('trash')}Purger le cache</button></div></div>
      <div class="stack">
        <div class="grid is-tiles">
          <div class="card tile"><div class="label">Cache média</div><div class="value">${bytes(server.cacheUsed)}</div><div class="foot" style="margin-top:8px">${meter(server.cacheUsed, CACHE_MAX, 'Cache')}sur ${bytes(CACHE_MAX)} · expiration 12 h</div></div>
          <div class="card tile"><div class="label">Boards (total)</div><div class="value">${bytes(total)}</div><div class="foot">${guilds.reduce((s, g) => s + g.board.items, 0)} médias · quota 15 Go par guilde</div></div>
          <div class="card tile"><div class="label">Disque du volume média</div><div class="value">71 <small>%</small></div><div class="foot" style="margin-top:8px">${meter(71, 100, 'Disque')}142 Go libres sur 500 Go</div></div>
          <div class="card tile"><div class="label">Fichiers orphelins</div><div class="value">3</div><div class="foot"><button class="btn is-small" data-act="toast" data-text="3 fichiers orphelins supprimés (212 Mo).">Nettoyer</button></div></div>
        </div>
        <section class="card"><div class="card-head"><h2>Occupation par guilde</h2></div><div class="table-wrap"><table class="tbl"><thead><tr><th>Guilde</th><th class="is-num">Médias</th><th>Occupation</th><th class="is-num">Quota</th><th><span class="sr-only">Actions</span></th></tr></thead><tbody>
          ${guilds.slice().sort((a, b) => b.board.usedBytes - a.board.usedBytes).map((guild) => `<tr><td><a class="primary" href="#/guilds/${guild.id}/board">${esc(guildName(guild))}</a></td><td class="is-num">${guild.board.items}</td>
            <td><span class="mini-meter" style="width:160px">${meter(guild.board.usedBytes, BOARD_MAX, 'Board')}</span><span class="num soft">${bytes(guild.board.usedBytes)} · ${Math.round((guild.board.usedBytes / BOARD_MAX) * 100)} %</span></td>
            <td class="is-num soft">${bytes(BOARD_MAX)}</td><td class="is-actions"><a class="btn is-small" href="#/guilds/${guild.id}/board">Gérer</a></td></tr>`).join('')}</tbody></table></div></section>
        <section class="card"><div class="card-head"><h2>Ingestions récentes</h2><span class="meta">téléchargement et transcodage</span></div><div class="table-wrap"><table class="tbl"><thead><tr><th>Quand</th><th>Média</th><th>Guilde</th><th>Origine</th><th>État</th><th>Détail</th></tr></thead><tbody>
          ${server.ingestions.map((item) => `<tr><td class="soft">${ago(item.min)}</td><td class="primary">${esc(item.title)}</td><td><a href="#/guilds/${item.guild.id}">${esc(guildName(item.guild))}</a></td><td class="soft">${esc(item.by)}</td><td><span class="badge ${INGEST[item.status][1]}">${INGEST[item.status][0]}</span></td><td class="soft is-wrap">${esc(item.detail)}</td></tr>`).join('')}</tbody></table></div></section>
      </div>`;
    return { nav: 'media', title: 'Médias et stockage', crumbs: crumb(['Médias et stockage']), html };
  }

  // ------------------------------------------------------- global: activity
  function activityPage() {
    regionRenderer = () => {
      const query = state.filters.logQuery.trim().toLowerCase();
      const rows = activity.filter((row) => !query || `${row.actor} ${row.action} ${row.route} ${row.guild ? guildName(row.guild) : ''}`.toLowerCase().includes(query));
      return rows.length ? `<div class="table-wrap">${activityTable(rows)}</div><p class="table-foot">${plural(rows.length, 'entrée', 'entrées')} · conservation 90 jours</p>` : '<div class="table-wrap"><p class="empty">Aucune entrée.</p></div>';
    };
    const html = `<div class="page-head"><div><h1>Journal</h1><p class="sub">Chaque action d’administration : qui, quoi, sur quelle guilde, avec quel résultat.</p></div>
        <div class="actions"><button class="btn" data-act="toast" data-text="Journal exporté (CSV).">Exporter en CSV</button></div></div>
      <div class="toolbar"><div class="search">${ic('search')}<input class="field" type="search" placeholder="Acteur, action, route ou guilde" aria-label="Rechercher dans le journal" data-filter="logQuery" value="${esc(state.filters.logQuery)}"></div></div>
      <div id="region"></div>`;
    return { nav: 'activity', title: 'Journal', crumbs: crumb(['Journal']), html };
  }

  // ------------------------------------------------------- global: settings
  function settingsPage() {
    // [label, control html, env name, hint]; `ro` marks what stays in the environment.
    const input = (id, value, extra) => `<input class="field" id="${id}" value="${esc(value)}" ${extra || ''}>`;
    const ro = (id, value) => `<input class="field mono" id="${id}" value="${esc(value)}" readonly aria-describedby="${id}-ro"><p class="hint" id="${id}-ro">Défini par l’environnement, modifiable au redémarrage.</p>`;
    const toggle = (id, label, checked) => `<label class="switch"><input type="checkbox" id="${id}"${checked ? ' checked' : ''}>${label}</label>`;
    const sections = [
      ['acces', 'Accès à l’administration', [
        ['Administrateurs (Discord)', `<textarea class="field mono" id="s-admins" rows="3">193847561029384756  # quentin\n284756102938475610  # maelle</textarea><p class="hint">Identifiants Discord autorisés à ouvrir cette console, un par ligne.</p>`, 'ADMIN_DISCORD_USER_IDS (nouveau)'],
        ['Secret de secours', `<div class="row"><input class="field mono" value="••••••••••••••••••••" readonly aria-label="Secret de secours"><button type="button" class="btn" data-act="rotate-secret">Régénérer</button></div>`, 'ADMIN_UI_TOKEN'],
        ['Réseau', toggle('s-local', 'Accès depuis cette machine uniquement', false), 'ADMIN_UI_LOCAL_ONLY'],
        ['Reverse proxy', toggle('s-fwd', 'Faire confiance à X-Forwarded-For', true), 'ADMIN_TRUST_FORWARDED_FOR'],
        ['Durée de session', `<select class="field" id="s-session"><option>12 heures</option><option>24 heures</option><option>7 jours</option></select>`, 'nouveau'],
      ]],
      ['discord', 'Discord', [
        ['Application', ro('s-client', '1570564356108714094'), 'DISCORD_CLIENT_ID'],
        ['Jeton du bot', `<div class="row"><span class="badge is-ok">Configuré</span><span class="soft">connecté en tant que LiveChat#4821</span></div>`, 'DISCORD_TOKEN'],
        ['Invitation', `<div class="row"><input class="field mono" value="${BOT_INVITE_URL}" readonly aria-label="Lien d’invitation du bot"><button type="button" class="btn is-icon" data-act="copy" data-text="${BOT_INVITE_URL}" aria-label="Copier le lien d’invitation">${ic('copy')}</button></div><p class="hint">Droits demandés : voir les salons, envoyer des messages, intégrer des liens.</p>`, ''],
        ['Commandes', toggle('s-hide', 'Masquer les commandes désactivées', false), 'HIDE_COMMANDS_DISABLED'],
        ['Langue du bot', `<select class="field" id="s-i18n"><option>Français</option><option>English</option></select>`, 'I18N'],
      ]],
      ['lecture', 'Lecture', [
        ['Durée par défaut (s)', input('s-duration', '5', 'type="number" min="1"'), 'DEFAULT_DURATION'],
        ['Historique des lectures (h)', input('s-retention', '24', 'type="number" min="1"'), 'PLAYBACK_JOB_RETENTION_HOURS'],
        ['Carte Twitter (s)', input('s-twitter', '15', 'type="number" min="1"'), 'TWITTER_CARD_DURATION_SEC'],
      ]],
      ['medias', 'Médias et quotas', [
        ['Cache : taille maximale (Mo)', input('s-cache', '5120', 'type="number"'), 'MEDIA_CACHE_MAX_TOTAL_MB'],
        ['Cache : durée de vie (h)', input('s-ttl', '12', 'type="number"'), 'MEDIA_CACHE_TTL_HOURS'],
        ['Board : quota par guilde (Mo)', input('s-board', '15360', 'type="number"'), 'MEDIA_BOARD_MAX_TOTAL_MB'],
        ['Taille maximale d’un média (Mo)', input('s-max', '200', 'type="number"'), 'MEDIA_MAX_SIZE_MB'],
        ['Hauteur vidéo maximale', `<select class="field" id="s-height"><option>720p</option><option selected>1080p</option><option>1440p</option></select>`, 'MEDIA_VIDEO_MAX_HEIGHT'],
        ['Encodeur vidéo', `<select class="field" id="s-encoder"><option>Automatique</option><option>libx264</option><option selected>h264_nvenc</option></select>`, 'MEDIA_VIDEO_ENCODER'],
        ['Volume', toggle('s-norm', 'Normaliser le volume (loudnorm, −16 LUFS)', true), 'MEDIA_AUDIO_NORMALIZE_ENABLED'],
        ['Ingestions simultanées', input('s-concurrent', '3', 'type="number" min="1" max="16"'), 'MEDIA_MAX_CONCURRENT_INGESTIONS'],
        ['Dossier de stockage', ro('s-dir', '/data/media'), 'MEDIA_STORAGE_DIR'],
      ]],
      ['sources', 'Sources et téléchargement', [
        ['Cookie TikTok', `<textarea class="field mono" id="s-tiktok" rows="2" placeholder="sessionid=…">sessionid=8f2c…; tt_csrf_token=…</textarea><p class="hint">Nécessaire pour les TikTok restreints. Appliqué immédiatement.</p>`, 'TIKTOK_COOKIE · runtime'],
        ['Format yt-dlp', input('s-ytdlp', 'bv*[height<=1080]+ba/b'), 'YTDLP_FORMAT'],
        ['Délai de téléchargement (ms)', input('s-timeout', '120000', 'type="number"'), 'MEDIA_DOWNLOAD_TIMEOUT_MS'],
        ['Binaires', `<dl class="kv"><dt>ffmpeg</dt><dd class="mono">7.1 · /usr/bin/ffmpeg</dd><dt>yt-dlp</dt><dd class="mono">2026.11.04 <button type="button" class="btn is-small" data-act="toast" data-text="yt-dlp est à jour.">Mettre à jour</button></dd></dl>`, 'FFMPEG_BINARY · YTDLP_BINARY'],
      ]],
      ['overlays', 'Overlays et appairage', [
        ['Version minimale', input('s-minver', MIN_OVERLAY_VERSION, 'class="field mono"') + '<p class="hint">Les overlays plus anciens sont refusés et invités à se mettre à jour.</p>', 'MIN_OVERLAY_VERSION'],
        ['Validité d’un code d’appairage (min)', input('s-pairttl', '10', 'type="number" min="1"'), 'PAIRING_CODE_TTL_MINUTES'],
        ['Délai de heartbeat WebSocket (s)', input('s-hb', '45', 'type="number" min="5"'), 'WS_HEARTBEAT_TIMEOUT_SEC'],
        ['Validité d’une URL de média (s)', input('s-urlttl', '3600', 'type="number"'), 'MEDIA_URL_TTL_SEC'],
      ]],
      ['tts', 'Synthèse vocale', [
        ['Moteur', ro('s-piper', '/opt/piper/piper'), 'PIPER_BIN'],
        ['Voix française', ro('s-voicefr', 'fr_FR-siwis-medium.onnx'), 'PIPER_VOICE_FR'],
        ['Voix anglaise', ro('s-voiceen', 'en_US-lessac-medium.onnx'), 'PIPER_VOICE_EN'],
      ]],
      ['reseau', 'Réseau', [
        ['URL publique', ro('s-api', 'https://livechat.example.net'), 'API_URL'],
        ['Port', ro('s-port', '3000'), 'PORT'],
        ['Origines CORS supplémentaires', `<textarea class="field mono" id="s-cors" rows="2">tauri://localhost</textarea>`, 'ADDITIONAL_CORS_ORIGINS'],
        ['Limitation de débit', toggle('s-trust', 'Lire l’IP cliente dans les en-têtes du proxy', true), 'TRUST_PROXY_HEADERS'],
        ['Base de données', ro('s-db', 'sqlite:///data/livechat.db'), 'DATABASE_URL'],
      ]],
    ];
    const html = `<div class="page-head"><div><h1>Paramètres du serveur</h1><p class="sub">Réglages de toute l’instance. Le nom de la variable d’environnement correspondante figure sous chaque champ.</p></div></div>
      <div class="settings-layout">
        <nav class="settings-nav" aria-label="Sections des paramètres">${sections.map(([id, title]) => `<a href="#/settings" data-act="scroll" data-target="set-${id}">${title}</a>`).join('')}</nav>
        <form class="stack" data-form="server-settings">
          ${sections
            .map(
              ([id, title, fields]) => `<section class="card" id="set-${id}"><div class="card-head"><h2>${title}</h2></div><div class="card-body"><div class="form">
              ${fields.map(([label, control, env]) => `<span class="lbl">${label}${env ? `<br><span class="env">${env}</span>` : ''}</span><div>${control}</div>`).join('')}
            </div></div></section>`,
            )
            .join('')}
          <div class="row"><button class="btn is-primary" type="submit">Enregistrer les paramètres</button><span class="soft">Les réglages marqués « environnement » demandent un redémarrage.</span></div>
        </form>
      </div>`;
    return { nav: 'settings', title: 'Paramètres du serveur', crumbs: crumb(['Paramètres du serveur']), html };
  }

  // ---------------------------------------------------------------- dialogs
  function permissionList(role, mode, guestsBoard) {
    return `<ul class="perms">${PERMISSIONS.map(([label, level]) => {
      const ok = permissionGranted(level, role, mode, guestsBoard);
      return `<li class="${ok ? '' : 'is-no'}">${ic(ok ? 'check' : 'minus')}<span>${label}<span class="sr-only">${ok ? ' : autorisé' : ' : refusé'}</span></span></li>`;
    }).join('')}</ul>`;
  }

  function deviceDialog(guild, device) {
    const member = memberOf(guild, device.memberId);
    const body = `
      <dl class="kv"><dt>Guilde</dt><dd>${esc(guildName(guild))}</dd><dt>Membre</dt><dd>${who(member.name)} <span class="badge ${member.role === 'admin' ? 'is-admin' : ''}">${ROLE_LABEL[member.role]}</span></dd>
        <dt>État</dt><dd>${device.connected ? '<span class="badge is-ok"><span class="dot is-ok"></span>En ligne</span>' : `Hors ligne, vu ${ago(device.lastSeen)}`}</dd>
        <dt>Version</dt><dd><span class="mono">${device.version}</span> · ${device.os}</dd><dt>Identifiant</dt><dd class="mono">${device.id}</dd></dl>
      <div class="dlg-section"><h3>Nom de l’appareil</h3><input class="field" id="dev-label" value="${esc(device.label)}" maxlength="120" aria-label="Nom de l’appareil"></div>
      <div class="dlg-section"><h3>Mode de session</h3>
        <label class="choice"><input type="radio" name="mode" value="normal"${device.sessionMode === 'normal' ? ' checked' : ''}><strong>Contrôle complet</strong><span>L’appareil a tous les droits du rôle de son membre (${ROLE_LABEL[member.role].toLowerCase()}).</span></label>
        <label class="choice"><input type="radio" name="mode" value="invite_read_only"${device.sessionMode === 'invite_read_only' ? ' checked' : ''}><strong>Lecture seule</strong><span>L’appareil reçoit les médias mais ne peut rien lancer, arrêter ni modifier, quel que soit le rôle du membre.</span></label></div>
      <div class="dlg-section"><h3>Droits effectifs de cet appareil</h3><div id="dev-perms">${permissionList(member.role, device.sessionMode, guild.settings.guestsCanViewBoard)}</div>
        <p class="hint">Pour changer les droits sur tous les appareils de ${esc(member.name)}, modifie son rôle dans l’onglet Membres.</p></div>`;
    openDialog(
      dialogShell({
        title: device.label,
        sub: 'Droits et session de l’overlay',
        body,
        foot: `<button type="button" class="btn is-danger left" data-act="revoke-device" data-guild="${guild.id}" data-id="${device.id}">Révoquer l’appareil</button>
               <button type="button" class="btn" data-act="close">Annuler</button><button type="submit" class="btn is-primary">Enregistrer</button>`,
      }),
      (form) => {
        device.sessionMode = form.elements.mode.value;
        device.label = document.getElementById('dev-label').value.trim() || device.label;
        dialog.close();
        toast(`${device.label} : session « ${MODE_LABEL[device.sessionMode]} ».${device.connected ? ' Appliqué sans reconnexion.' : ''}`);
        render();
      },
      true,
    );
    dialog.querySelectorAll('input[name="mode"]').forEach((radio) =>
      radio.addEventListener('change', () => {
        document.getElementById('dev-perms').innerHTML = permissionList(member.role, radio.value, guild.settings.guestsCanViewBoard);
      }),
    );
  }

  function secretDialog(title, sub, value, hint) {
    openDialog(
      dialogShell({
        title,
        sub,
        body: `<div class="secret"><span>${esc(value)}</span><button type="button" class="btn is-small" data-act="copy" data-text="${esc(value)}">${ic('copy')}Copier</button></div><p class="hint">${hint}</p>`,
        foot: '<button type="submit" class="btn is-primary">Terminé</button>',
      }),
      () => dialog.close(),
    );
  }

  function createGuildDialog() {
    openDialog(
      dialogShell({
        title: 'Créer une guilde indépendante',
        sub: 'Un groupe sans serveur Discord. Pour un serveur Discord, il suffit d’y inviter le bot.',
        body: `<label for="ng-name">Nom de la guilde</label><input class="field" id="ng-name" maxlength="60" required style="margin-top:6px" placeholder="Soirée jeux du vendredi">
          <p class="hint">Un code d’invitation admin, valable 24 h, est créé en même temps : transmets-le à la personne qui gérera la guilde.</p>`,
        foot: '<button type="button" class="btn" data-act="close">Annuler</button><button type="submit" class="btn is-primary">Créer la guilde</button>',
      }),
      () => {
        const name = document.getElementById('ng-name').value.trim();
        if (!name) {
          return;
        }
        const guild = buildGuild({ id: `9000000000000000${10 + guilds.length}`, name, kind: 'none', members: 0, devices: 0, invites: 0, pairing: 0, ingest: 0, board: 0, boardGb: 0, createdDays: 0, lastActivity: 0 });
        guilds.push(guild);
        location.hash = `#/guilds/${guild.id}`;
        secretDialog('Guilde créée', `Code d’invitation admin pour « ${esc(name)} »`, `${code().slice(0, 4)}-${code().slice(0, 4)}`, 'À saisir dans l’overlay : Rejoindre une guilde. Valable 24 h, une seule utilisation.');
      },
    );
  }

  function inviteDialog(guild) {
    const select = (id, label, options) => `<div><label for="${id}">${label}</label><select class="field" id="${id}" style="margin-top:6px">${options.map(([value, text]) => `<option value="${value}">${text}</option>`).join('')}</select></div>`;
    openDialog(
      dialogShell({
        title: 'Créer une invitation',
        sub: esc(guildName(guild)),
        body: `<div class="stack">${select('inv-role', 'Rôle accordé', [['member', 'Membre'], ['guest', 'Invité'], ['admin', 'Admin']])}
          ${select('inv-exp', 'Expiration', [['24', 'Dans 24 h'], ['168', 'Dans 7 jours'], ['', 'Jamais']])}
          ${select('inv-uses', 'Nombre d’utilisations', [['1', '1 utilisation'], ['10', '10 utilisations'], ['', 'Illimité']])}</div>`,
        foot: '<button type="button" class="btn" data-act="close">Annuler</button><button type="submit" class="btn is-primary">Créer le code</button>',
      }),
      () => {
        const invite = { code: code(), role: document.getElementById('inv-role').value, createdBy: state.user.name, createdMin: 0, expiresHours: Number(document.getElementById('inv-exp').value) || null, maxUses: Number(document.getElementById('inv-uses').value) || null, useCount: 0, status: 'active' };
        guild.invites.unshift(invite);
        render();
        secretDialog('Invitation créée', `${ROLE_LABEL[invite.role]} · ${esc(guildName(guild))}`, `${invite.code.slice(0, 4)}-${invite.code.slice(4)}`, 'À saisir dans l’overlay : Rejoindre une guilde.');
      },
    );
  }

  function ingestDialog() {
    openDialog(
      dialogShell({
        title: 'Créer un client d’ingestion',
        sub: 'Pour une extension de navigateur ou un script.',
        body: `<div class="stack"><div><label for="ic-guild">Guilde</label><select class="field" id="ic-guild" style="margin-top:6px">${guilds.map((guild) => `<option value="${guild.id}">${esc(guildName(guild))}</option>`).join('')}</select></div>
          <div><label for="ic-author">Auteur par défaut</label><select class="field" id="ic-author" style="margin-top:6px">${guilds[0].members.map((member) => `<option>${esc(member.name)}</option>`).join('')}</select><p class="hint">Les médias envoyés par ce client seront signés de ce nom.</p></div>
          <div><label for="ic-label">Libellé</label><input class="field" id="ic-label" maxlength="120" value="Chrome" style="margin-top:6px"></div></div>`,
        foot: '<button type="button" class="btn" data-act="close">Annuler</button><button type="submit" class="btn is-primary">Créer le client</button>',
      }),
      () => {
        const guild = findGuild(document.getElementById('ic-guild').value);
        const author = document.getElementById('ic-author').value;
        guild.ingest.push({ id: uuid(), label: `${document.getElementById('ic-label').value.trim() || 'Client'}-${author}`, authorName: author, lastSeen: null, createdDays: 0 });
        render();
        secretDialog('Client créé', 'Jeton d’ingestion', hex(64), 'Ce jeton n’est affiché qu’une seule fois : copie-le maintenant dans l’extension.');
      },
    );
  }

  // ---------------------------------------------------------------- actions
  const actions = {
    close: () => dialog.close(),
    logout: () => {
      setUser(null);
      location.hash = '#/login';
      render();
    },
    'login-discord': () => {
      setUser({ name: 'quentin', via: 'Connecté avec Discord' });
      location.hash = '#/dashboard';
      render();
    },
    copy: (el) => {
      if (navigator.clipboard) {
        navigator.clipboard.writeText(el.dataset.text).catch(() => {});
      }
      toast('Copié dans le presse-papiers.');
    },
    toast: (el) => toast(el.dataset.text),
    scroll: (el, event) => {
      event.preventDefault();
      const target = document.getElementById(el.dataset.target);
      target.scrollIntoView({ block: 'start' });
    },
    'filter-outdated': () => {
      state.filters.deviceState = 'outdated';
    },
    'create-guild': createGuildDialog,
    'create-invite': (el, event, guild) => inviteDialog(guild),
    'create-ingest': ingestDialog,
    device: (el, event, guild) => deviceDialog(guild, guild.devices.find((device) => device.id === el.dataset.id)),
    'admin-code': (el, event, guild) => secretDialog('Code admin', `Accès administrateur à « ${esc(guildName(guild))} »`, `${code().slice(0, 4)}-${code().slice(0, 4)}`, 'Valable 24 h, une seule utilisation. Utile quand une guilde n’a plus d’admin.'),
    'device-code': (el, event, guild) => secretDialog('Code d’appairage', `Nouvel overlay pour « ${esc(guildName(guild))} »`, code(), 'À saisir dans l’overlay. Valable 10 minutes.'),
    'link-code': (el, event, guild) => secretDialog('Code de liaison Discord', esc(guildName(guild)), `/lier code:${code()}`, 'Un administrateur du serveur Discord tape cette commande. Valable 10 minutes.'),
    'rotate-secret': () =>
      confirmDialog({ title: 'Régénérer le secret de secours ?', body: 'L’ancien secret cesse de fonctionner tout de suite. Les sessions Discord ne sont pas touchées.', label: 'Régénérer', onConfirm: () => secretDialog('Nouveau secret', 'Secret d’administration de secours', hex(48), 'Affiché une seule fois. Range-le dans ton gestionnaire de mots de passe.') }),
    stop: (el, event, guild) => {
      guild.queue.shift();
      toast(`Lecture arrêtée sur ${guildName(guild)}.`);
      render();
    },
    'clear-queue': (el, event, guild) =>
      confirmDialog({ title: 'Vider la file d’attente ?', body: `Les ${guild.queue.length - 1} médias en attente sont retirés. La lecture en cours continue.`, label: 'Vider la file', onConfirm: () => { guild.queue.length = 1; toast('File vidée.'); render(); } }),
    'remove-job': (el, event, guild) => {
      guild.queue = guild.queue.filter((job) => job.id !== el.dataset.id);
      toast('Média retiré de la file.');
      render();
    },
    kick: (el, event, guild) => {
      const member = memberOf(guild, el.dataset.id);
      confirmDialog({
        title: `Exclure ${member.name} ?`,
        body: 'Ses overlays sont déconnectés tout de suite. Il lui faudra un nouveau code pour revenir.',
        label: 'Exclure',
        onConfirm: () => {
          guild.members = guild.members.filter((item) => item !== member);
          guild.devices = guild.devices.filter((device) => device.memberId !== member.id);
          toast(`${member.name} a été exclu.`);
          render();
        },
      });
    },
    'revoke-device': (el, event, guild) => {
      const device = guild.devices.find((item) => item.id === el.dataset.id);
      confirmDialog({
        title: `Révoquer ${device.label} ?`,
        body: 'L’overlay est déconnecté et son jeton invalidé. Le membre reste dans la guilde et peut appairer un nouvel appareil.',
        label: 'Révoquer l’appareil',
        onConfirm: () => {
          guild.devices = guild.devices.filter((item) => item !== device);
          toast(`${device.label} révoqué.`);
          render();
        },
      });
    },
    'revoke-invite': (el, event, guild) => {
      guild.invites.find((invite) => invite.code === el.dataset.id).status = 'revoked';
      toast('Invitation révoquée.');
      render();
    },
    'revoke-code': (el, event, guild) => {
      guild.pairing.find((item) => item.code === el.dataset.id).status = 'expired';
      toast('Code d’appairage révoqué.');
      render();
    },
    'purge-codes': (el, event, guild) => {
      let count = 0;
      (guild ? [guild] : guilds).forEach((item) => {
        count += item.pairing.filter((p) => p.status !== 'active').length + item.invites.filter((i) => i.status !== 'active').length;
        item.pairing = item.pairing.filter((p) => p.status === 'active');
        item.invites = item.invites.filter((i) => i.status === 'active');
      });
      toast(`${plural(count, 'code supprimé', 'codes supprimés')}.`);
      render();
    },
    'revoke-ingest': (el, event, guild) => {
      const client = guild.ingest.find((item) => item.id === el.dataset.id);
      confirmDialog({ title: `Révoquer ${client.label} ?`, body: 'Son jeton cesse de fonctionner tout de suite.', label: 'Révoquer', onConfirm: () => { guild.ingest = guild.ingest.filter((item) => item !== client); toast(`${client.label} révoqué.`); render(); } });
    },
    'delete-media': (el, event, guild) => {
      const [item] = guild.board.recent.splice(Number(el.dataset.id), 1);
      guild.board.items -= 1;
      guild.board.usedBytes -= item.size;
      toast(`« ${item.title} » supprimé du board.`);
      render();
    },
    'clear-board': (el, event, guild) =>
      confirmDialog({
        title: 'Vider le board média ?',
        body: `Les ${guild.board.items} médias de « ${esc(guildName(guild))} » et leurs fichiers (${bytes(guild.board.usedBytes)}) sont supprimés. Irréversible.`,
        label: 'Vider le board',
        expect: guild.id,
        onConfirm: () => { Object.assign(guild.board, { items: 0, usedBytes: 0, favorites: 0, recent: [], kinds: { video: 0, audio: 0, image: 0 } }); toast('Board vidé.'); render(); },
      }),
    'revoke-all': (el, event, guild) =>
      confirmDialog({
        title: 'Révoquer tous les accès ?',
        body: `${plural(guild.devices.length, 'overlay', 'overlays')}, ${plural(guild.ingest.length, 'client d’ingestion', 'clients d’ingestion')} et ${plural(guild.members.length, 'membre', 'membres')} perdent l’accès à « ${esc(guildName(guild))} ». Un code admin permettra de repartir.`,
        label: 'Révoquer les accès',
        expect: guild.id,
        onConfirm: () => { Object.assign(guild, { devices: [], ingest: [], members: [], invites: [], pairing: [], queue: [] }); toast('Tous les accès ont été révoqués.'); render(); },
      }),
    'unlink-discord': (el, event, guild) =>
      confirmDialog({
        title: 'Délier le serveur Discord ?',
        body: 'Les commandes de ce serveur Discord ne viseront plus cette guilde.',
        label: 'Délier',
        extra: '<div class="dlg-section"><label class="switch"><input type="checkbox" id="opt-leave">Faire aussi quitter le bot de ce serveur</label></div>',
        onConfirm: () => { guild.kind = 'none'; guild.discordServerId = null; guild.botPresent = false; toast('Serveur Discord délié.'); render(); },
      }),
    'leave-discord': (el, event, guild) =>
      confirmDialog({ title: 'Faire quitter le bot ?', body: `Le bot quitte le serveur Discord de « ${esc(guildName(guild))} ». Les commandes n’y fonctionneront plus ; les données restent sur le serveur LiveChat.`, label: 'Faire quitter le bot', onConfirm: () => { guild.botPresent = false; toast('Le bot a quitté le serveur Discord.'); render(); } }),
    'delete-guild': (el, event, guild) =>
      confirmDialog({
        title: `Supprimer « ${guildName(guild)} » ?`,
        body: 'Membres, overlays, codes, file d’attente, board et réglages sont supprimés définitivement.',
        label: 'Supprimer la guilde',
        expect: guild.id,
        extra: `<div class="dlg-section stack"><label class="switch"><input type="checkbox" checked>Supprimer aussi les fichiers médias orphelins</label>${guild.kind !== 'none' ? '<br><label class="switch"><input type="checkbox">Faire quitter le bot du serveur Discord</label>' : ''}</div>`,
        onConfirm: () => { guilds.splice(guilds.indexOf(guild), 1); toast(`Guilde « ${guildName(guild)} » supprimée.`); location.hash = '#/guilds'; },
      }),
    'purge-cache': () =>
      confirmDialog({ title: 'Purger le cache média ?', body: `${bytes(server.cacheUsed)} de médias hors board sont supprimés. Ceux des lectures en cours sont conservés ; les autres seront retéléchargés à la demande.`, label: 'Purger le cache', onConfirm: () => { server.cacheUsed = 180 * MIB; toast('Cache purgé.'); render(); } }),
  };

  // ----------------------------------------------------------------- events
  document.addEventListener('click', (event) => {
    const seg = event.target.closest('[data-seg]');
    if (seg) {
      state.filters[seg.dataset.seg] = seg.dataset.value;
      seg.parentElement.querySelectorAll('button').forEach((button) => button.setAttribute('aria-pressed', String(button === seg)));
      refreshRegion();
      return;
    }
    const el = event.target.closest('[data-act]');
    if (!el || el.disabled) {
      return;
    }
    const action = actions[el.dataset.act];
    if (action) {
      action(el, event, el.dataset.guild ? findGuild(el.dataset.guild) : null);
    }
  });

  document.addEventListener('input', (event) => {
    const target = event.target;
    if (target.dataset.filter) {
      state.filters[target.dataset.filter] = target.value;
      refreshRegion();
    }
    if (target.dataset.expect) {
      document.getElementById('confirm-go').disabled = target.value.trim() !== target.dataset.expect;
    }
  });

  document.addEventListener('change', (event) => {
    const target = event.target;
    if (target.dataset.change === 'role') {
      const guild = findGuild(target.dataset.guild);
      const member = memberOf(guild, target.dataset.id);
      member.role = target.value;
      toast(`${member.name} est maintenant ${ROLE_LABEL[member.role].toLowerCase()}.`);
    }
  });

  document.addEventListener('submit', (event) => {
    const form = event.target;
    event.preventDefault();
    if (form.hasAttribute('data-dialog-form')) {
      if (dialogSubmit) {
        dialogSubmit(form);
      }
      return;
    }
    if (form.dataset.form === 'login-secret') {
      setUser({ name: 'Secret admin', via: 'Connecté avec le secret d’administration' });
      location.hash = '#/dashboard';
      render();
    } else if (form.dataset.form === 'guild-settings') {
      const guild = findGuild(form.dataset.guild);
      const number = (id) => Number(document.getElementById(id).value) || null;
      guild.name = document.getElementById('gs-name').value.trim();
      Object.assign(guild.settings, { defaultMediaTime: number('gs-default'), maxMediaTime: number('gs-max'), displayMediaFull: document.getElementById('gs-full').checked, guestsCanViewBoard: document.getElementById('gs-guests').checked });
      toast('Réglages enregistrés et diffusés aux overlays connectés.');
      render();
    } else if (form.dataset.form === 'server-settings') {
      toast('Paramètres du serveur enregistrés.');
    }
  });

  window.addEventListener('hashchange', () => {
    if (dialog.open && !dialog.querySelector('.secret')) {
      dialog.close();
    }
    render();
    window.scrollTo(0, 0);
  });

  if (!state.user && route()[0] !== 'login') {
    location.hash = '#/login';
  }
  render();
})();
