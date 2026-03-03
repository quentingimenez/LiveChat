import { timingSafeEqual } from 'crypto';
import type { FastifyReply, FastifyRequest } from 'fastify';
import { executeManualStopForGuild } from '../../services/manualStop';
import { MediaAssetStatus, PlaybackJobStatus } from '../../services/prisma/prismaEnums';

const BYTES_PER_MEGABYTE = 1024 * 1024;

interface AdminGuildSettingsBody {
  defaultMediaTime?: unknown;
  maxMediaTime?: unknown;
  displayMediaFull?: unknown;
}

interface PairingCodesQuery {
  guildId?: unknown;
  status?: unknown;
}

interface PurgePairingCodesQuery {
  guildId?: unknown;
}

interface OverlayClientRecord {
  id: string;
  guildId: string;
  label: string;
  createdAt: Date;
  lastSeenAt: Date | null;
  revokedAt: Date | null;
}

interface IngestClientRecord {
  id: string;
  guildId: string;
  label: string;
  createdAt: Date;
  lastSeenAt: Date | null;
  revokedAt: Date | null;
}

interface PairingCodeRecord {
  code: string;
  guildId: string;
  createdByDiscordUserId: string;
  authorName: string | null;
  expiresAt: Date;
  usedAt: Date | null;
  createdAt: Date;
}

interface GuildRecord {
  id: string;
  busyUntil: Date | null;
  defaultMediaTime: number | null;
  maxMediaTime: number | null;
  displayMediaFull: boolean;
}

interface GroupedPlaybackCount {
  guildId: string;
  status: string;
  _count: {
    _all: number;
  };
}

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const toOptionalGuildSettingInt = (value: unknown): number | null | undefined => {
  if (value === undefined) {
    return undefined;
  }

  if (value === null || value === '') {
    return null;
  }

  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return undefined;
  }

  const normalized = Math.floor(value);
  if (normalized <= 0) {
    return null;
  }

  return normalized;
};

const toOptionalBoolean = (value: unknown): boolean | undefined => {
  if (value === undefined) {
    return undefined;
  }

  if (typeof value !== 'boolean') {
    return undefined;
  }

  return value;
};

const toSafeSizeBytes = (value: unknown): number => {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    return 0;
  }

  return Math.floor(value);
};

const toPercent = (value: number, max: number): number => {
  if (max <= 0) {
    return 0;
  }

  return Math.max(0, Math.min(100, Number(((value / max) * 100).toFixed(2))));
};

const toLoopbackIp = (rawIp: string): string => {
  const normalized = rawIp.trim().toLowerCase();

  if (normalized.startsWith('::ffff:')) {
    return normalized.slice('::ffff:'.length);
  }

  return normalized;
};

const isLoopbackIp = (ip: string): boolean => {
  const normalized = toLoopbackIp(ip);

  return normalized === '127.0.0.1' || normalized === '::1' || normalized === '0:0:0:0:0:0:0:1';
};

const safeTokenEquals = (expected: string, candidate: string): boolean => {
  const expectedBuffer = Buffer.from(expected);
  const candidateBuffer = Buffer.from(candidate);

  if (expectedBuffer.length !== candidateBuffer.length) {
    return false;
  }

  return timingSafeEqual(expectedBuffer, candidateBuffer);
};

const getRequestToken = (request: FastifyRequest): string | null => {
  const authHeader = request.headers.authorization;
  const queryToken = toNonEmptyString((request.query as Record<string, unknown> | undefined)?.token);

  if (authHeader) {
    const [scheme, token] = authHeader.split(' ');
    if (scheme?.toLowerCase() === 'bearer' && toNonEmptyString(token)) {
      return token.trim();
    }
  }

  return queryToken;
};

const assertAdminAccess = async (request: FastifyRequest, reply: FastifyReply): Promise<boolean> => {
  const expectedToken = env.ADMIN_UI_TOKEN.trim();

  if (!expectedToken) {
    await reply.code(503).send({
      error: 'admin_ui_disabled',
      message: 'ADMIN_UI_TOKEN is not configured',
    });
    return false;
  }

  if (env.ADMIN_UI_LOCAL_ONLY && !isLoopbackIp(request.ip)) {
    await reply.code(403).send({
      error: 'admin_local_only',
    });
    return false;
  }

  const requestToken = getRequestToken(request);

  if (!requestToken || !safeTokenEquals(expectedToken, requestToken)) {
    await reply.code(401).send({
      error: 'unauthorized',
    });
    return false;
  }

  return true;
};

const getIngestClientDelegate = (): {
  findMany: (args: unknown) => Promise<unknown>;
  updateMany: (args: unknown) => Promise<unknown>;
  findFirst: (args: unknown) => Promise<unknown>;
} | null => {
  const delegate = (prisma as unknown as { ingestClient?: unknown }).ingestClient;

  if (!delegate || typeof delegate !== 'object') {
    return null;
  }

  return delegate as {
    findMany: (args: unknown) => Promise<unknown>;
    updateMany: (args: unknown) => Promise<unknown>;
    findFirst: (args: unknown) => Promise<unknown>;
  };
};

const listIngestClients = async (): Promise<IngestClientRecord[]> => {
  const delegate = getIngestClientDelegate();
  if (!delegate) {
    return [];
  }

  try {
    const records = await delegate.findMany({
      where: {
        revokedAt: null,
      },
      orderBy: [
        {
          guildId: 'asc',
        },
        {
          label: 'asc',
        },
      ],
    });

    return records as IngestClientRecord[];
  } catch (error) {
    logger.warn({ err: error }, '[ADMIN] Unable to list ingest clients');
    return [];
  }
};

const revokeIngestClient = async (clientId: string) => {
  const delegate = getIngestClientDelegate();
  if (!delegate) {
    return {
      found: false,
      revoked: false,
    };
  }

  const existing = (await delegate.findFirst({
    where: {
      id: clientId,
    },
    select: {
      id: true,
      guildId: true,
      label: true,
      revokedAt: true,
    },
  })) as { id?: string; guildId?: string; label?: string; revokedAt?: Date | null } | null;

  if (!existing?.id) {
    return {
      found: false,
      revoked: false,
    };
  }

  const updateResult = (await delegate.updateMany({
    where: {
      id: clientId,
      revokedAt: null,
    },
    data: {
      revokedAt: new Date(),
    },
  })) as { count?: number };

  return {
    found: true,
    revoked: (updateResult.count || 0) > 0,
    guildId: existing.guildId || null,
    label: existing.label || null,
  };
};

const resolveDiscordGuildName = async (guildId: string): Promise<string | null> => {
  const client = global.discordClient;

  if (!client) {
    return null;
  }

  const cachedGuild = client.guilds.cache.get(guildId);
  if (cachedGuild?.name) {
    return cachedGuild.name;
  }

  try {
    const fetchedGuild = await client.guilds.fetch(guildId);
    return fetchedGuild?.name || null;
  } catch {
    return null;
  }
};

const collectConnectedOverlayClientIds = async (
  fastify: FastifyCustomInstance,
  guildIds: string[],
): Promise<Map<string, Set<string>>> => {
  const result = new Map<string, Set<string>>();

  await Promise.all(
    guildIds.map(async (guildId) => {
      const roomName = `overlay-guild-${guildId}`;
      const sockets = await fastify.io.in(roomName).fetchSockets();
      const connectedClientIds = new Set<string>();

      for (const socket of sockets) {
        const overlayClientId = toNonEmptyString(socket.data?.overlayClientId);
        if (overlayClientId) {
          connectedClientIds.add(overlayClientId);
        }
      }

      result.set(guildId, connectedClientIds);
    }),
  );

  return result;
};

const buildAdminPanelHtml = () => {
  return `<!doctype html>
<html lang="fr">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>LiveChat Admin</title>
    <style>
      :root {
        --bg-0: #0c111d;
        --bg-1: #111b2f;
        --bg-2: #1b2a44;
        --panel: rgba(17, 27, 47, 0.84);
        --panel-border: rgba(128, 170, 230, 0.24);
        --text: #f7fbff;
        --muted: #a8b8d3;
        --ok: #38d39f;
        --warn: #f5b759;
        --danger: #ff6767;
        --accent: #67c2ff;
        --accent-2: #8d87ff;
        --shadow: 0 14px 32px rgba(2, 8, 24, 0.45);
      }

      * {
        box-sizing: border-box;
      }

      body {
        margin: 0;
        min-height: 100vh;
        color: var(--text);
        font-family: 'IBM Plex Sans', 'Segoe UI', sans-serif;
        background:
          radial-gradient(1200px 700px at 12% -10%, rgba(103, 194, 255, 0.26), transparent 60%),
          radial-gradient(900px 600px at 100% 0%, rgba(141, 135, 255, 0.2), transparent 55%),
          linear-gradient(160deg, var(--bg-0), var(--bg-1) 45%, var(--bg-2));
      }

      .layout {
        width: min(1280px, 96vw);
        margin: 20px auto 30px;
        display: grid;
        gap: 14px;
      }

      .panel {
        background: var(--panel);
        border: 1px solid var(--panel-border);
        border-radius: 14px;
        padding: 14px;
        box-shadow: var(--shadow);
        backdrop-filter: blur(4px);
      }

      h1,
      h2,
      h3 {
        margin: 0;
        line-height: 1.2;
      }

      h1 {
        font-size: 24px;
      }

      h2 {
        font-size: 18px;
      }

      h3 {
        font-size: 14px;
      }

      .muted {
        color: var(--muted);
      }

      .topbar {
        display: grid;
        grid-template-columns: 1fr auto;
        gap: 12px;
        align-items: end;
      }

      .token-row {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
        align-items: center;
      }

      .token-row input,
      .token-row select,
      .token-row button,
      .guild-settings input {
        border-radius: 10px;
        border: 1px solid rgba(255, 255, 255, 0.2);
        background: rgba(7, 12, 24, 0.82);
        color: var(--text);
        padding: 8px 10px;
        font-size: 13px;
      }

      .token-row input,
      .token-row select {
        min-width: 170px;
      }

      .token-row button,
      .action-btn {
        cursor: pointer;
        transition: transform 0.15s ease, border-color 0.15s ease;
      }

      .token-row button:hover,
      .action-btn:hover {
        transform: translateY(-1px);
        border-color: rgba(103, 194, 255, 0.7);
      }

      .status {
        min-height: 18px;
        font-size: 13px;
      }

      .status.ok {
        color: var(--ok);
      }

      .status.warn {
        color: var(--warn);
      }

      .status.error {
        color: var(--danger);
      }

      .metrics-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
        gap: 10px;
        margin-top: 10px;
      }

      .metric {
        border-radius: 12px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        background: rgba(6, 10, 22, 0.55);
        padding: 10px;
      }

      .metric strong {
        display: block;
        font-size: 21px;
        margin-bottom: 4px;
      }

      .guilds {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
        gap: 10px;
      }

      .guild-card {
        border-radius: 12px;
        border: 1px solid rgba(255, 255, 255, 0.14);
        background: rgba(7, 12, 24, 0.58);
        padding: 10px;
        display: grid;
        gap: 9px;
      }

      .guild-head {
        display: flex;
        justify-content: space-between;
        gap: 8px;
        align-items: flex-start;
      }

      .badge {
        display: inline-block;
        border-radius: 999px;
        padding: 2px 8px;
        font-size: 11px;
        border: 1px solid rgba(255, 255, 255, 0.24);
      }

      .badge.ok {
        color: var(--ok);
        border-color: rgba(56, 211, 159, 0.5);
      }

      .badge.warn {
        color: var(--warn);
        border-color: rgba(245, 183, 89, 0.52);
      }

      .guild-grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 8px;
      }

      .guild-meta,
      .list-block {
        border-radius: 10px;
        background: rgba(8, 14, 28, 0.58);
        border: 1px solid rgba(255, 255, 255, 0.12);
        padding: 8px;
      }

      .guild-meta div {
        font-size: 12px;
        margin-bottom: 4px;
      }

      .guild-settings {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 8px;
        align-items: end;
      }

      .guild-settings label {
        font-size: 12px;
        display: grid;
        gap: 4px;
      }

      .guild-settings .check {
        display: flex;
        align-items: center;
        gap: 6px;
      }

      .actions {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
      }

      .action-btn {
        border-radius: 9px;
        border: 1px solid rgba(255, 255, 255, 0.2);
        background: rgba(9, 15, 29, 0.85);
        color: var(--text);
        padding: 7px 10px;
        font-size: 12px;
      }

      .action-btn.warn {
        border-color: rgba(245, 183, 89, 0.6);
      }

      .action-btn.danger {
        border-color: rgba(255, 103, 103, 0.65);
      }

      .list-block ul {
        list-style: none;
        padding: 0;
        margin: 8px 0 0;
        display: grid;
        gap: 6px;
      }

      .list-item {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 6px;
        font-size: 12px;
        display: grid;
        gap: 4px;
      }

      .pairing-controls {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
        align-items: center;
      }

      table {
        width: 100%;
        border-collapse: collapse;
        margin-top: 10px;
      }

      th,
      td {
        text-align: left;
        font-size: 12px;
        padding: 7px 6px;
        border-bottom: 1px solid rgba(255, 255, 255, 0.1);
      }

      td code {
        font-family: 'IBM Plex Mono', 'Consolas', monospace;
      }

      .progress {
        height: 8px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.1);
        overflow: hidden;
        margin-top: 6px;
      }

      .progress > span {
        display: block;
        height: 100%;
        background: linear-gradient(90deg, var(--accent), var(--accent-2));
      }

      @media (max-width: 920px) {
        .topbar {
          grid-template-columns: 1fr;
        }

        .guild-settings {
          grid-template-columns: repeat(2, minmax(0, 1fr));
        }
      }

      @media (max-width: 640px) {
        .guild-grid {
          grid-template-columns: 1fr;
        }

        .guild-settings {
          grid-template-columns: 1fr;
        }
      }
    </style>
  </head>
  <body>
    <div class="layout">
      <section class="panel">
        <div class="topbar">
          <div>
            <h1>LiveChat Admin Panel</h1>
            <div class="muted">Accès local uniquement. Token requis.</div>
          </div>
          <div class="token-row">
            <input id="token-input" type="password" placeholder="ADMIN_UI_TOKEN" />
            <button id="save-token">Enregistrer token</button>
            <button id="refresh-overview">Rafraîchir</button>
          </div>
        </div>
        <div id="status" class="status muted">Prêt.</div>
        <div id="metrics" class="metrics-grid"></div>
      </section>

      <section class="panel">
        <h2>Guilds</h2>
        <div class="muted">Vue globale des guilds, overlays, ingest, files et réglages.</div>
        <div id="guilds" class="guilds" style="margin-top: 10px"></div>
      </section>

      <section class="panel">
        <h2>Pairing Codes</h2>
        <div class="pairing-controls" style="margin-top: 8px">
          <select id="pairing-guild-filter"></select>
          <select id="pairing-status-filter">
            <option value="all">Tous</option>
            <option value="active">Actifs</option>
            <option value="expired">Expirés / utilisés</option>
          </select>
          <button id="pairing-load">Charger</button>
          <button id="pairing-purge" class="action-btn warn">Purger expirés</button>
        </div>
        <div id="pairing-summary" class="status muted" style="margin-top: 8px"></div>
        <table>
          <thead>
            <tr>
              <th>Code</th>
              <th>Guild</th>
              <th>Auteur</th>
              <th>Expire</th>
              <th>Utilisé</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody id="pairing-rows"></tbody>
        </table>
      </section>
    </div>

    <script>
      const state = {
        token: '',
        overview: null,
        pairingItems: [],
      };

      const escapeHtml = (value) => {
        return String(value || '')
          .replace(/&/g, '&amp;')
          .replace(/</g, '&lt;')
          .replace(/>/g, '&gt;')
          .replace(/\"/g, '&quot;')
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
                return (
                  '<li class="list-item">' +
                  '<div><strong>' +
                  escapeHtml(client.label) +
                  '</strong> <span class="badge ' +
                  badgeClass +
                  '">' +
                  badgeText +
                  '</span></div>' +
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
              '<div class="guild-grid">' +
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
              '">' +
              '<label>Default media time (sec)<input type="number" min="1" name="defaultMediaTime" value="' +
              escapeHtml(guild.settings.defaultMediaTime == null ? '' : String(guild.settings.defaultMediaTime)) +
              '" /></label>' +
              '<label>Max media time (sec)<input type="number" min="1" name="maxMediaTime" value="' +
              escapeHtml(guild.settings.maxMediaTime == null ? '' : String(guild.settings.maxMediaTime)) +
              '" /></label>' +
              '<label class="check"><input type="checkbox" name="displayMediaFull" ' +
              (guild.settings.displayMediaFull ? 'checked' : '') +
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
              '</div>' +
              '<div class="guild-grid">' +
              '<div class="list-block"><strong>Overlay clients</strong><ul>' +
              (overlayItems || '<li class="list-item muted">Aucun client</li>') +
              '</ul></div>' +
              '<div class="list-block"><strong>Ingest clients</strong><ul>' +
              (ingestItems || '<li class="list-item muted">Aucun client</li>') +
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

      const submitGuildSettings = async (form) => {
        const guildId = form.getAttribute('data-guild-id');
        if (!guildId) {
          return;
        }

        const data = new FormData(form);

        const defaultMediaTimeRaw = String(data.get('defaultMediaTime') || '').trim();
        const maxMediaTimeRaw = String(data.get('maxMediaTime') || '').trim();

        const payload = {
          defaultMediaTime: defaultMediaTimeRaw ? Number(defaultMediaTimeRaw) : null,
          maxMediaTime: maxMediaTimeRaw ? Number(maxMediaTimeRaw) : null,
          displayMediaFull: data.get('displayMediaFull') === 'on',
        };

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
      };

      const refreshAll = async () => {
        try {
          setStatus('Chargement des données admin...', 'muted');
          await loadOverview();
          await loadPairingCodes();
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
        if (action !== 'save-settings') {
          return;
        }

        event.preventDefault();

        try {
          await submitGuildSettings(form);
        } catch (error) {
          setStatus('Erreur sauvegarde settings: ' + (error instanceof Error ? error.message : 'request_failed'), 'error');
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
        setInterval(refreshAll, 10000);
      };

      boot();
    </script>
  </body>
</html>`;
};

export const AdminRoutes = () =>
  async function (fastify: FastifyCustomInstance) {
    fastify.get('/', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      reply.type('text/html; charset=utf-8');
      return reply.send(buildAdminPanelHtml());
    });

    fastify.get('/api/overview', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const [guildRecords, overlayClients, ingestClients, pairingCodes, boardItems, playbackGrouped, cacheAggregate] =
        await Promise.all([
          prisma.guild.findMany({
            orderBy: {
              id: 'asc',
            },
          }) as Promise<GuildRecord[]>,
          prisma.overlayClient.findMany({
            where: {
              revokedAt: null,
            },
            orderBy: [
              {
                guildId: 'asc',
              },
              {
                label: 'asc',
              },
            ],
          }) as Promise<OverlayClientRecord[]>,
          listIngestClients(),
          prisma.pairingCode.findMany({
            orderBy: {
              createdAt: 'desc',
            },
          }) as Promise<PairingCodeRecord[]>,
          prisma.memeBoardItem.findMany({
            include: {
              mediaAsset: {
                select: {
                  sizeBytes: true,
                },
              },
            },
          }) as Promise<Array<{ guildId: string; mediaAsset: { sizeBytes: number } }>>,
          prisma.playbackJob.groupBy({
            by: ['guildId', 'status'],
            _count: {
              _all: true,
            },
          }) as Promise<GroupedPlaybackCount[]>,
          prisma.mediaAsset.aggregate({
            where: {
              status: MediaAssetStatus.READY,
              memeBoardItems: {
                none: {},
              },
            },
            _sum: {
              sizeBytes: true,
            },
          }),
        ]);

      const guildIds = new Set<string>();

      for (const guildRecord of guildRecords) {
        guildIds.add(guildRecord.id);
      }

      for (const overlayClient of overlayClients) {
        guildIds.add(overlayClient.guildId);
      }

      for (const ingestClient of ingestClients) {
        guildIds.add(ingestClient.guildId);
      }

      for (const boardItem of boardItems) {
        guildIds.add(boardItem.guildId);
      }

      for (const grouped of playbackGrouped) {
        guildIds.add(grouped.guildId);
      }

      for (const pairingCode of pairingCodes) {
        guildIds.add(pairingCode.guildId);
      }

      const discordGuildCache = global.discordClient?.guilds?.cache;
      if (discordGuildCache) {
        for (const guildId of discordGuildCache.keys()) {
          guildIds.add(guildId);
        }
      }

      const sortedGuildIds = Array.from(guildIds.values()).sort((a, b) =>
        a.localeCompare(b, undefined, {
          sensitivity: 'base',
        }),
      );

      const [connectedOverlayMap, discordNamePairs] = await Promise.all([
        collectConnectedOverlayClientIds(fastify, sortedGuildIds),
        Promise.all(
          sortedGuildIds.map(async (guildId) => {
            const name = await resolveDiscordGuildName(guildId);
            return [guildId, name] as const;
          }),
        ),
      ]);

      const guildNameMap = new Map<string, string | null>(discordNamePairs);
      const guildRecordMap = new Map<string, GuildRecord>(guildRecords.map((guildRecord) => [guildRecord.id, guildRecord]));

      const overlaysByGuild = new Map<string, OverlayClientRecord[]>();
      for (const overlayClient of overlayClients) {
        const existing = overlaysByGuild.get(overlayClient.guildId) || [];
        existing.push(overlayClient);
        overlaysByGuild.set(overlayClient.guildId, existing);
      }

      const ingestByGuild = new Map<string, IngestClientRecord[]>();
      for (const ingestClient of ingestClients) {
        const existing = ingestByGuild.get(ingestClient.guildId) || [];
        existing.push(ingestClient);
        ingestByGuild.set(ingestClient.guildId, existing);
      }

      const boardBytesByGuild = new Map<string, number>();
      const boardCountByGuild = new Map<string, number>();

      for (const boardItem of boardItems) {
        const guildId = boardItem.guildId;
        const nextBytes = (boardBytesByGuild.get(guildId) || 0) + toSafeSizeBytes(boardItem.mediaAsset?.sizeBytes);
        boardBytesByGuild.set(guildId, nextBytes);
        boardCountByGuild.set(guildId, (boardCountByGuild.get(guildId) || 0) + 1);
      }

      const playbackByGuild = new Map<string, { pending: number; playing: number; failed: number; done: number }>();
      for (const grouped of playbackGrouped) {
        const current = playbackByGuild.get(grouped.guildId) || {
          pending: 0,
          playing: 0,
          failed: 0,
          done: 0,
        };

        const count = grouped._count._all;
        if (grouped.status === PlaybackJobStatus.PENDING) {
          current.pending = count;
        } else if (grouped.status === PlaybackJobStatus.PLAYING) {
          current.playing = count;
        } else if (grouped.status === PlaybackJobStatus.FAILED) {
          current.failed = count;
        } else if (grouped.status === PlaybackJobStatus.DONE) {
          current.done = count;
        }

        playbackByGuild.set(grouped.guildId, current);
      }

      const now = new Date();
      let pairingActiveCount = 0;
      let pairingExpiredCount = 0;

      for (const pairingCode of pairingCodes) {
        const isActive = !pairingCode.usedAt && pairingCode.expiresAt > now;
        if (isActive) {
          pairingActiveCount += 1;
        } else {
          pairingExpiredCount += 1;
        }
      }

      const cacheMaxBytes = Math.max(1, env.MEDIA_CACHE_MAX_TOTAL_MB) * BYTES_PER_MEGABYTE;
      const cacheUsedBytes = toSafeSizeBytes(cacheAggregate._sum.sizeBytes);
      const cacheRemainingBytes = Math.max(0, cacheMaxBytes - cacheUsedBytes);
      const boardMaxBytes = Math.max(1, env.MEDIA_BOARD_MAX_TOTAL_MB) * BYTES_PER_MEGABYTE;

      const guildsPayload = sortedGuildIds.map((guildId) => {
        const guildRecord = guildRecordMap.get(guildId) || null;
        const guildName = guildNameMap.get(guildId);
        const guildOverlays = overlaysByGuild.get(guildId) || [];
        const guildIngest = ingestByGuild.get(guildId) || [];
        const connectedIds = connectedOverlayMap.get(guildId) || new Set<string>();
        const boardUsedBytes = boardBytesByGuild.get(guildId) || 0;
        const playback = playbackByGuild.get(guildId) || {
          pending: 0,
          playing: 0,
          failed: 0,
          done: 0,
        };

        return {
          id: guildId,
          name: guildName || `Guild ${guildId}`,
          busyUntil: guildRecord?.busyUntil || null,
          settings: {
            defaultMediaTime: guildRecord?.defaultMediaTime ?? null,
            maxMediaTime: guildRecord?.maxMediaTime ?? null,
            displayMediaFull: guildRecord?.displayMediaFull ?? false,
          },
          overlays: {
            total: guildOverlays.length,
            connectedCount: connectedIds.size,
            clients: guildOverlays.map((client) => ({
              id: client.id,
              label: client.label,
              lastSeenAt: client.lastSeenAt,
              createdAt: client.createdAt,
              connected: connectedIds.has(client.id),
            })),
          },
          ingest: {
            total: guildIngest.length,
            clients: guildIngest.map((client) => ({
              id: client.id,
              label: client.label,
              lastSeenAt: client.lastSeenAt,
              createdAt: client.createdAt,
            })),
          },
          memeBoard: {
            itemsCount: boardCountByGuild.get(guildId) || 0,
            usedBytes: boardUsedBytes,
            maxBytes: boardMaxBytes,
            remainingBytes: Math.max(0, boardMaxBytes - boardUsedBytes),
            usedPercent: toPercent(boardUsedBytes, boardMaxBytes),
          },
          playback,
        };
      });

      const overlayConnectedCount = guildsPayload.reduce((acc, guild) => acc + guild.overlays.connectedCount, 0);

      return reply.send({
        generatedAt: new Date().toISOString(),
        cache: {
          usedBytes: cacheUsedBytes,
          maxBytes: cacheMaxBytes,
          remainingBytes: cacheRemainingBytes,
          usedPercent: toPercent(cacheUsedBytes, cacheMaxBytes),
        },
        totals: {
          guildCount: guildsPayload.length,
          overlayClientCount: overlayClients.length,
          overlayConnectedCount,
          ingestClientCount: ingestClients.length,
          pairingCodeCount: pairingCodes.length,
        },
        pairingCodes: {
          active: pairingActiveCount,
          expired: pairingExpiredCount,
          total: pairingCodes.length,
        },
        guilds: guildsPayload,
      });
    });

    fastify.patch<{ Params: { guildId: string }; Body: AdminGuildSettingsBody }>(
      '/api/guilds/:guildId/settings',
      async (request, reply) => {
        if (!(await assertAdminAccess(request, reply))) {
          return;
        }

        const guildId = toNonEmptyString(request.params.guildId);

        if (!guildId) {
          return reply.code(400).send({
            error: 'invalid_guild_id',
          });
        }

        const defaultMediaTime = toOptionalGuildSettingInt(request.body?.defaultMediaTime);
        const maxMediaTime = toOptionalGuildSettingInt(request.body?.maxMediaTime);
        const displayMediaFull = toOptionalBoolean(request.body?.displayMediaFull);

        const hasDefault = defaultMediaTime !== undefined;
        const hasMax = maxMediaTime !== undefined;
        const hasDisplay = displayMediaFull !== undefined;

        if (!hasDefault && !hasMax && !hasDisplay) {
          return reply.code(400).send({
            error: 'invalid_payload',
          });
        }

        if ((request.body?.defaultMediaTime !== undefined && defaultMediaTime === undefined) ||
            (request.body?.maxMediaTime !== undefined && maxMediaTime === undefined) ||
            (request.body?.displayMediaFull !== undefined && displayMediaFull === undefined)) {
          return reply.code(400).send({
            error: 'invalid_payload',
          });
        }

        const createData: Record<string, unknown> = {
          id: guildId,
        };
        const updateData: Record<string, unknown> = {};

        if (hasDefault) {
          createData.defaultMediaTime = defaultMediaTime;
          updateData.defaultMediaTime = defaultMediaTime;
        }

        if (hasMax) {
          createData.maxMediaTime = maxMediaTime;
          updateData.maxMediaTime = maxMediaTime;
        }

        if (hasDisplay) {
          createData.displayMediaFull = displayMediaFull;
          updateData.displayMediaFull = displayMediaFull;
        }

        const guild = (await prisma.guild.upsert({
          where: {
            id: guildId,
          },
          create: createData,
          update: updateData,
        })) as GuildRecord;

        return reply.send({
          updated: true,
          guild: {
            id: guild.id,
            defaultMediaTime: guild.defaultMediaTime,
            maxMediaTime: guild.maxMediaTime,
            displayMediaFull: guild.displayMediaFull,
            busyUntil: guild.busyUntil,
          },
        });
      },
    );

    fastify.post<{ Params: { guildId: string } }>('/api/guilds/:guildId/stop', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const guildId = toNonEmptyString(request.params.guildId);

      if (!guildId) {
        return reply.code(400).send({
          error: 'invalid_guild_id',
        });
      }

      const result = await executeManualStopForGuild(fastify, guildId, {
        logLabel: 'Admin panel stop',
      });

      return reply.send({
        stopped: true,
        guildId,
        releasedCount: result.releasedCount,
      });
    });

    fastify.post<{ Params: { clientId: string } }>('/api/overlay-clients/:clientId/revoke', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const clientId = toNonEmptyString(request.params.clientId);
      if (!clientId) {
        return reply.code(400).send({
          error: 'invalid_client_id',
        });
      }

      const existing = await prisma.overlayClient.findFirst({
        where: {
          id: clientId,
        },
        select: {
          id: true,
          guildId: true,
          label: true,
          revokedAt: true,
        },
      });

      if (!existing) {
        return reply.code(404).send({
          error: 'overlay_client_not_found',
        });
      }

      const updateResult = await prisma.overlayClient.updateMany({
        where: {
          id: clientId,
          revokedAt: null,
        },
        data: {
          revokedAt: new Date(),
        },
      });

      return reply.send({
        found: true,
        revoked: updateResult.count > 0,
        clientId,
        guildId: existing.guildId,
        label: existing.label,
      });
    });

    fastify.post<{ Params: { clientId: string } }>('/api/ingest-clients/:clientId/revoke', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const clientId = toNonEmptyString(request.params.clientId);
      if (!clientId) {
        return reply.code(400).send({
          error: 'invalid_client_id',
        });
      }

      const revokeResult = await revokeIngestClient(clientId);

      if (!revokeResult.found) {
        return reply.code(404).send({
          error: 'ingest_client_not_found',
        });
      }

      return reply.send({
        found: true,
        revoked: revokeResult.revoked,
        clientId,
        guildId: revokeResult.guildId || null,
        label: revokeResult.label || null,
      });
    });

    fastify.get<{ Querystring: PairingCodesQuery }>('/api/pairing-codes', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const guildId = toNonEmptyString(request.query?.guildId);
      const status = toNonEmptyString(request.query?.status) || 'all';
      const now = new Date();

      const records = (await prisma.pairingCode.findMany({
        where: {
          ...(guildId
            ? {
                guildId,
              }
            : {}),
        },
        orderBy: {
          createdAt: 'desc',
        },
      })) as PairingCodeRecord[];

      const items = records
        .map((record) => {
          const isActive = !record.usedAt && record.expiresAt > now;
          return {
            code: record.code,
            guildId: record.guildId,
            createdByDiscordUserId: record.createdByDiscordUserId,
            authorName: record.authorName,
            expiresAt: record.expiresAt,
            usedAt: record.usedAt,
            createdAt: record.createdAt,
            isActive,
          };
        })
        .filter((record) => {
          if (status === 'active') {
            return record.isActive;
          }

          if (status === 'expired') {
            return !record.isActive;
          }

          return true;
        });

      const active = items.filter((item) => item.isActive).length;

      return reply.send({
        items,
        counts: {
          total: items.length,
          active,
          expired: items.length - active,
        },
      });
    });

    fastify.delete<{ Querystring: PurgePairingCodesQuery }>('/api/pairing-codes/expired', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const guildId = toNonEmptyString(request.query?.guildId);

      const deleteResult = await prisma.pairingCode.deleteMany({
        where: {
          ...(guildId
            ? {
                guildId,
              }
            : {}),
          OR: [
            {
              usedAt: {
                not: null,
              },
            },
            {
              expiresAt: {
                lte: new Date(),
              },
            },
          ],
        },
      });

      return reply.send({
        deletedCount: deleteResult.count,
        guildId: guildId || null,
      });
    });

    fastify.post<{ Params: { code: string } }>('/api/pairing-codes/:code/revoke', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const code = toNonEmptyString(request.params.code)?.toUpperCase();
      if (!code) {
        return reply.code(400).send({
          error: 'invalid_code',
        });
      }

      const existing = await prisma.pairingCode.findFirst({
        where: {
          code,
        },
        select: {
          code: true,
          usedAt: true,
          expiresAt: true,
          guildId: true,
        },
      });

      if (!existing) {
        return reply.code(404).send({
          error: 'pairing_code_not_found',
        });
      }

      const now = new Date();
      const updateResult = await prisma.pairingCode.updateMany({
        where: {
          code,
          usedAt: null,
          expiresAt: {
            gt: now,
          },
        },
        data: {
          usedAt: now,
          expiresAt: now,
        },
      });

      return reply.send({
        found: true,
        revoked: updateResult.count > 0,
        guildId: existing.guildId,
      });
    });
  };
