import { timingSafeEqual } from 'crypto';
import { rm } from 'fs/promises';
import type { FastifyReply, FastifyRequest } from 'fastify';
import { createIngestClientToken } from '../../services/ingestAuth';
import { executeManualStopForGuild } from '../../services/manualStop';
import { MediaAssetStatus, PlaybackJobStatus } from '../../services/prisma/prismaEnums';
import { getRuntimeTikTokCookie, persistRuntimeTikTokCookieToEnvFile } from '../../services/runtimeSettings';
import { disconnectOverlayClient } from '../../loaders/socketLoader';
import { toNonEmptyString } from '../../services/stringUtils';
import { normalizeOverlaySessionMode, type OverlayClientRecord } from '../../services/overlayAuth';
import { type IngestClientRecord } from '../../services/ingestAuth';
import { type OverlaySessionMode } from '@livechat/overlay-protocol';

const BYTES_PER_MEGABYTE = 1024 * 1024;

interface AdminGuildSettingsBody {
  defaultMediaTime?: unknown;
  maxMediaTime?: unknown;
  displayMediaFull?: unknown;
}


interface AdminGuildPurgeBody {
  confirmGuildId?: unknown;
  removeOrphanMedia?: unknown;
}

interface AdminRuntimeSettingsBody {
  tiktokCookie?: unknown;
}

interface AdminCreateIngestClientBody {
  guildId?: unknown;
  authorDiscordUserId?: unknown;
  label?: unknown;
}

interface PairingCodesQuery {
  guildId?: unknown;
  status?: unknown;
}

interface PurgePairingCodesQuery {
  guildId?: unknown;
}


interface ConnectedOverlayClientState {
  clientId: string;
  sessionMode: OverlaySessionMode;
}


interface KnownIngestAuthor {
  discordUserId: string;
  displayName: string;
  image: string | null;
  lastSeenAt: Date | null;
  source: 'pairing' | 'ingest' | 'mixed';
}

interface PairingCodeRecord {
  code: string;
  guildId: string;
  createdByDiscordUserId: string;
  authorName: string | null;
  expiresAt: Date | null;
  usedAt: Date | null;
  createdAt: Date;
}

interface GuildRecord {
  id: string;
  name?: string | null;
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

const isPairingCodeActive = (
  pairingCode: {
    usedAt: Date | null;
    expiresAt: Date | null;
  },
  now: Date,
) => {
  if (pairingCode.usedAt) {
    return false;
  }

  if (!pairingCode.expiresAt) {
    return true;
  }

  return pairingCode.expiresAt > now;
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

  return timingSafeEqual(new Uint8Array(expectedBuffer), new Uint8Array(candidateBuffer));
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
  deleteMany: (args: unknown) => Promise<unknown>;
} | null => {
  const delegate = (prisma as unknown as { ingestClient?: unknown }).ingestClient;

  if (!delegate || typeof delegate !== 'object') {
    return null;
  }

  return delegate as {
    findMany: (args: unknown) => Promise<unknown>;
    updateMany: (args: unknown) => Promise<unknown>;
    findFirst: (args: unknown) => Promise<unknown>;
    deleteMany: (args: unknown) => Promise<unknown>;
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

const mergeAuthorSource = (
  current: KnownIngestAuthor['source'],
  incoming: KnownIngestAuthor['source'],
): KnownIngestAuthor['source'] => {
  if (current === incoming) {
    return current;
  }

  return 'mixed';
};

const listKnownIngestAuthors = async (): Promise<KnownIngestAuthor[]> => {
  const delegate = getIngestClientDelegate();
  const [pairingRecords, ingestRecords] = await Promise.all([
    prisma.pairingCode.findMany({
      select: {
        createdByDiscordUserId: true,
        authorName: true,
        authorImage: true,
        createdAt: true,
      },
      orderBy: {
        createdAt: 'desc',
      },
    }) as Promise<Array<{ createdByDiscordUserId: string; authorName: string | null; authorImage: string | null; createdAt: Date }>>,
    delegate
      ? (delegate.findMany({
          select: {
            createdByDiscordUserId: true,
            defaultAuthorName: true,
            defaultAuthorImage: true,
            createdAt: true,
          },
          orderBy: {
            createdAt: 'desc',
          },
        }) as Promise<
          Array<{
            createdByDiscordUserId: string;
            defaultAuthorName: string;
            defaultAuthorImage: string | null;
            createdAt: Date;
          }>
        >)
      : Promise.resolve([]),
  ]);

  const byDiscordUserId = new Map<string, KnownIngestAuthor>();

  const upsertKnownAuthor = (params: {
    discordUserId: string;
    displayName: string | null;
    image: string | null;
    createdAt: Date;
    source: 'pairing' | 'ingest';
  }) => {
    const existing = byDiscordUserId.get(params.discordUserId);
    const normalizedName = toNonEmptyString(params.displayName) || null;
    const normalizedImage = toNonEmptyString(params.image) || null;
    const safeDate = params.createdAt instanceof Date ? params.createdAt : new Date(0);

    if (!existing) {
      byDiscordUserId.set(params.discordUserId, {
        discordUserId: params.discordUserId,
        displayName: normalizedName || params.discordUserId,
        image: normalizedImage,
        lastSeenAt: safeDate,
        source: params.source,
      });
      return;
    }

    existing.source = mergeAuthorSource(existing.source, params.source);

    if (!existing.lastSeenAt || safeDate > existing.lastSeenAt) {
      existing.lastSeenAt = safeDate;
      existing.displayName = normalizedName || existing.displayName || params.discordUserId;
      existing.image = normalizedImage || existing.image || null;
      return;
    }

    if ((!existing.displayName || existing.displayName === params.discordUserId) && normalizedName) {
      existing.displayName = normalizedName;
    }

    if (!existing.image && normalizedImage) {
      existing.image = normalizedImage;
    }
  };

  pairingRecords.forEach((record) => {
    const discordUserId = toNonEmptyString(record.createdByDiscordUserId);
    if (!discordUserId) {
      return;
    }

    upsertKnownAuthor({
      discordUserId,
      displayName: toNonEmptyString(record.authorName),
      image: toNonEmptyString(record.authorImage),
      createdAt: record.createdAt,
      source: 'pairing',
    });
  });

  ingestRecords.forEach((record) => {
    const discordUserId = toNonEmptyString(record.createdByDiscordUserId);
    if (!discordUserId) {
      return;
    }

    upsertKnownAuthor({
      discordUserId,
      displayName: toNonEmptyString(record.defaultAuthorName),
      image: toNonEmptyString(record.defaultAuthorImage),
      createdAt: record.createdAt,
      source: 'ingest',
    });
  });

  return Array.from(byDiscordUserId.values())
    .sort((left, right) => {
      const byName = left.displayName.localeCompare(right.displayName, undefined, {
        sensitivity: 'base',
      });
      if (byName !== 0) {
        return byName;
      }

      return left.discordUserId.localeCompare(right.discordUserId, undefined, {
        sensitivity: 'base',
      });
    });
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

const toDeleteManyCount = (value: unknown): number => {
  if (!value || typeof value !== 'object') {
    return 0;
  }

  const count = (value as { count?: unknown }).count;
  if (typeof count !== 'number' || !Number.isFinite(count) || count < 0) {
    return 0;
  }

  return Math.floor(count);
};

const listGuildReferencedMediaAssetIds = async (guildId: string): Promise<string[]> => {
  const [boardRows, jobRows] = await Promise.all([
    prisma.memeBoardItem.findMany({
      where: {
        guildId,
      },
      select: {
        mediaAssetId: true,
      },
    }),
    prisma.playbackJob.findMany({
      where: {
        guildId,
        mediaAssetId: {
          not: null,
        },
      },
      select: {
        mediaAssetId: true,
      },
    }),
  ]);

  const mediaAssetIds = new Set<string>();

  for (const row of boardRows) {
    const mediaAssetId = toNonEmptyString(row.mediaAssetId);
    if (mediaAssetId) {
      mediaAssetIds.add(mediaAssetId);
    }
  }

  for (const row of jobRows) {
    const mediaAssetId = toNonEmptyString(row.mediaAssetId);
    if (mediaAssetId) {
      mediaAssetIds.add(mediaAssetId);
    }
  }

  return Array.from(mediaAssetIds.values());
};

const purgeOrphanMediaAssets = async (mediaAssetIds: string[]): Promise<{ deletedCount: number; fileDeleteErrors: number }> => {
  const normalizedIds = Array.from(new Set(mediaAssetIds.map((id) => toNonEmptyString(id)).filter((id): id is string => !!id)));

  if (normalizedIds.length === 0) {
    return {
      deletedCount: 0,
      fileDeleteErrors: 0,
    };
  }

  const candidates = await prisma.mediaAsset.findMany({
    where: {
      id: {
        in: normalizedIds,
      },
      memeBoardItems: {
        none: {},
      },
      playbackJobs: {
        none: {},
      },
    },
    select: {
      id: true,
      storagePath: true,
    },
  });

  let deletedCount = 0;
  let fileDeleteErrors = 0;

  for (const candidate of candidates) {
    if (candidate.storagePath) {
      try {
        await rm(candidate.storagePath, { force: true });
      } catch (error) {
        fileDeleteErrors += 1;
        logger.warn(
          { err: error, mediaAssetId: candidate.id, storagePath: candidate.storagePath },
          '[ADMIN] Failed to delete media file while purging guild',
        );
      }
    }

    const deleteResult = await prisma.mediaAsset.deleteMany({
      where: {
        id: candidate.id,
      },
    });
    deletedCount += deleteResult.count;
  }

  return {
    deletedCount,
    fileDeleteErrors,
  };
};

const disconnectOverlaySocketsForGuild = async (fastify: FastifyCustomInstance, guildId: string): Promise<number> => {
  const roomName = `overlay-guild-${guildId}`;
  const sockets = await fastify.io.in(roomName).fetchSockets();

  for (const socket of sockets) {
    socket.disconnect(true);
  }

  return sockets.length;
};

const collectConnectedOverlayClientIds = async (
  fastify: FastifyCustomInstance,
  guildIds: string[],
): Promise<Map<string, Map<string, ConnectedOverlayClientState>>> => {
  const result = new Map<string, Map<string, ConnectedOverlayClientState>>();

  await Promise.all(
    guildIds.map(async (guildId) => {
      const roomName = `overlay-guild-${guildId}`;
      const sockets = await fastify.io.in(roomName).fetchSockets();
      const connectedClients = new Map<string, ConnectedOverlayClientState>();

      for (const socket of sockets) {
        const overlayClientId = toNonEmptyString(socket.data?.overlayClientId);
        if (overlayClientId) {
          const sessionMode = normalizeOverlaySessionMode(socket.data?.overlaySessionMode);
          const existing = connectedClients.get(overlayClientId);

          if (!existing) {
            connectedClients.set(overlayClientId, {
              clientId: overlayClientId,
              sessionMode,
            });
            continue;
          }

          if (sessionMode === 'invite_read_only' && existing.sessionMode !== 'invite_read_only') {
            existing.sessionMode = sessionMode;
          }
        }
      }

      result.set(guildId, connectedClients);
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
        grid-template-columns: 1fr;
        gap: 10px;
      }

      .guild-card {
        border-radius: 12px;
        border: 1px solid rgba(255, 255, 255, 0.14);
        background: rgba(7, 12, 24, 0.58);
        padding: 9px;
        display: grid;
        gap: 8px;
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

      .badge.info {
        color: var(--accent);
        border-color: rgba(103, 194, 255, 0.52);
      }

      .guild-grid {
        display: grid;
        gap: 8px;
      }

      .guild-grid-meta {
        grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
      }

      .guild-grid-clients {
        grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
        align-items: start;
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
        margin: 6px 0 0;
        display: grid;
        gap: 5px;
      }

      .list-item {
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        padding: 6px;
        font-size: 12px;
        display: grid;
        gap: 4px;
      }

      .overlay-client-head {
        display: flex;
        align-items: center;
        gap: 8px;
      }

      .overlay-avatar {
        width: 24px;
        height: 24px;
        border-radius: 999px;
        object-fit: cover;
        border: 1px solid rgba(255, 255, 255, 0.18);
      }

      .list-item.empty {
        border-style: dashed;
        text-align: center;
        display: block;
        padding: 7px;
        color: var(--muted);
        background: rgba(6, 10, 22, 0.46);
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

        .guild-name-form {
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
        <h2>Runtime Settings</h2>
        <div class="muted">Appliqué immédiatement et persisté dans le fichier .env.</div>
        <div class="token-row" style="margin-top: 8px">
          <input id="tiktok-cookie-input" type="password" placeholder="TIKTOK_COOKIE (header Cookie TikTok)" />
          <button id="save-tiktok-cookie">Enregistrer TIKTOK_COOKIE</button>
          <button id="clear-tiktok-cookie" class="action-btn warn">Vider</button>
        </div>
        <div id="tiktok-cookie-status" class="status muted" style="margin-top: 8px">TIKTOK_COOKIE non chargé.</div>
      </section>

      <section class="panel">
        <h2>Create Ingest Client</h2>
        <div class="muted">Créer un token ingest lié à une guild et un auteur déjà présent en DB.</div>
        <div class="pairing-controls" style="margin-top: 8px">
          <select id="ingest-create-guild"></select>
          <select id="ingest-create-author"></select>
          <input id="ingest-create-label" type="text" placeholder="Label appareil (obligatoire)" />
          <button id="ingest-create-submit">Créer client ingest</button>
        </div>
        <div class="token-row" style="margin-top: 8px">
          <input id="ingest-create-token" type="text" readonly placeholder="Nouveau token ingest (copiez-le et stockez-le)." />
          <button id="ingest-create-copy-token">Copier token</button>
        </div>
        <div id="ingest-create-status" class="status muted" style="margin-top: 8px">Aucun client créé.</div>
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
                      '" alt="Avatar" referrerpolicy="no-referrer" loading="lazy" onerror="this.style.display=&apos;none&apos;" />'
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
            'Action irréversible. Tapez l\\'ID exact de la guild pour confirmer la purge complète.',
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
        const isActive = isPairingCodeActive(pairingCode, now);
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
        const guildName = toNonEmptyString(guildNameMap.get(guildId));
        const persistedGuildName = toNonEmptyString(guildRecord?.name);
        const effectiveGuildName = guildName || persistedGuildName || `Guild ${guildId}`;
        const editableGuildName = guildName || persistedGuildName || '';
        const guildOverlays = overlaysByGuild.get(guildId) || [];
        const guildIngest = ingestByGuild.get(guildId) || [];
        const connectedClients = connectedOverlayMap.get(guildId) || new Map<string, ConnectedOverlayClientState>();
        const boardUsedBytes = boardBytesByGuild.get(guildId) || 0;
        const playback = playbackByGuild.get(guildId) || {
          pending: 0,
          playing: 0,
          failed: 0,
          done: 0,
        };
        const hasGuildOverride = !!guildRecord;
        const effectiveDefaultMediaTime = guildRecord?.defaultMediaTime ?? env.DEFAULT_DURATION;
        const effectiveMaxMediaTime = guildRecord?.maxMediaTime ?? null;
        const effectiveDisplayMediaFull = guildRecord?.displayMediaFull ?? false;

        return {
          id: guildId,
          name: effectiveGuildName,
          editableName: editableGuildName,
          busyUntil: guildRecord?.busyUntil || null,
          settings: {
            defaultMediaTime: effectiveDefaultMediaTime,
            maxMediaTime: effectiveMaxMediaTime,
            displayMediaFull: effectiveDisplayMediaFull,
            overrides: {
              defaultMediaTime: guildRecord?.defaultMediaTime ?? null,
              maxMediaTime: guildRecord?.maxMediaTime ?? null,
              displayMediaFull: hasGuildOverride ? guildRecord.displayMediaFull : null,
            },
          },
          overlays: {
            total: guildOverlays.length,
            connectedCount: connectedClients.size,
            clients: guildOverlays.map((client) => {
              const persistedSessionMode = normalizeOverlaySessionMode(client.sessionMode);
              return {
                id: client.id,
                label: client.label,
                authorName: toNonEmptyString(client.defaultAuthorName),
                authorImage: toNonEmptyString(client.defaultAuthorImage),
                lastSeenAt: client.lastSeenAt,
                createdAt: client.createdAt,
                connected: connectedClients.has(client.id),
                sessionMode: connectedClients.get(client.id)?.sessionMode || persistedSessionMode,
              };
            }),
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

    fastify.get('/api/runtime-settings', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const tiktokCookie = getRuntimeTikTokCookie();
      return reply.send({
        tiktokCookie,
        hasTikTokCookie: tiktokCookie.length > 0,
      });
    });

    fastify.patch<{ Body: AdminRuntimeSettingsBody }>('/api/runtime-settings', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      if (request.body?.tiktokCookie === undefined || typeof request.body?.tiktokCookie !== 'string') {
        return reply.code(400).send({
          error: 'invalid_payload',
        });
      }

      const tiktokCookie = await persistRuntimeTikTokCookieToEnvFile(request.body.tiktokCookie);

      return reply.send({
        updated: true,
        tiktokCookie,
        hasTikTokCookie: tiktokCookie.length > 0,
      });
    });

    fastify.get('/api/ingest-authors', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const items = await listKnownIngestAuthors();

      return reply.send({
        items: items.map((item) => ({
          discordUserId: item.discordUserId,
          displayName: item.displayName,
          image: item.image,
          lastSeenAt: item.lastSeenAt,
          source: item.source,
        })),
        count: items.length,
      });
    });

    fastify.post<{ Body: AdminCreateIngestClientBody }>('/api/ingest-clients', async (request, reply) => {
      if (!(await assertAdminAccess(request, reply))) {
        return;
      }

      const guildId = toNonEmptyString(request.body?.guildId);
      const authorDiscordUserId = toNonEmptyString(request.body?.authorDiscordUserId);
      const label = toNonEmptyString(request.body?.label);

      if (!guildId) {
        return reply.code(400).send({
          error: 'invalid_guild_id',
        });
      }

      if (!authorDiscordUserId) {
        return reply.code(400).send({
          error: 'invalid_author_discord_user_id',
        });
      }

      if (!label) {
        return reply.code(400).send({
          error: 'invalid_label',
        });
      }

      const knownAuthors = await listKnownIngestAuthors();
      const selectedAuthor = knownAuthors.find((author) => author.discordUserId === authorDiscordUserId);

      if (!selectedAuthor) {
        return reply.code(400).send({
          error: 'author_not_found',
        });
      }

      const authorSuffix = toNonEmptyString(selectedAuthor.displayName) || selectedAuthor.discordUserId;
      const deviceLabel = `${label}-${authorSuffix}`;
      const { client, rawToken } = await createIngestClientToken({
        guildId,
        label: deviceLabel,
        defaultAuthorName: selectedAuthor.displayName,
        defaultAuthorImage: selectedAuthor.image,
        createdByDiscordUserId: selectedAuthor.discordUserId,
      });

      return reply.send({
        created: true,
        ingestApiToken: rawToken,
        client: {
          id: client.id,
          guildId: client.guildId,
          label: client.label,
          defaultAuthorName: client.defaultAuthorName,
          defaultAuthorImage: client.defaultAuthorImage,
          createdByDiscordUserId: client.createdByDiscordUserId,
          createdAt: client.createdAt,
        },
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

    fastify.post<{ Params: { guildId: string }; Body: AdminGuildPurgeBody }>(
      '/api/guilds/:guildId/purge',
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

        const confirmGuildId = toNonEmptyString(request.body?.confirmGuildId);
        if (!confirmGuildId || confirmGuildId !== guildId) {
          return reply.code(400).send({
            error: 'confirm_guild_id_mismatch',
          });
        }

        const removeOrphanMediaRaw = request.body?.removeOrphanMedia;
        if (removeOrphanMediaRaw !== undefined && typeof removeOrphanMediaRaw !== 'boolean') {
          return reply.code(400).send({
            error: 'invalid_payload',
          });
        }
        const removeOrphanMedia = removeOrphanMediaRaw !== false;

        const candidateMediaAssetIds = removeOrphanMedia ? await listGuildReferencedMediaAssetIds(guildId) : [];
        const stopResult = await executeManualStopForGuild(fastify, guildId, {
          logLabel: 'Admin panel purge',
        });

        const disconnectedOverlaySockets = await disconnectOverlaySocketsForGuild(fastify, guildId);
        const ingestDelegate = getIngestClientDelegate();
        let ingestDeleteCount = 0;
        if (ingestDelegate) {
          try {
            const ingestDeleteRaw = await ingestDelegate.deleteMany({
              where: {
                guildId,
              },
            });
            ingestDeleteCount = toDeleteManyCount(ingestDeleteRaw);
          } catch (error) {
            logger.warn({ err: error, guildId }, '[ADMIN] Unable to delete ingest clients while purging guild');
          }
        }

        const [overlayDeleteResult, pairingDeleteResult, playbackDeleteResult, boardDeleteResult, guildDeleteResult] = await Promise.all([
          prisma.overlayClient.deleteMany({
            where: {
              guildId,
            },
          }),
          prisma.pairingCode.deleteMany({
            where: {
              guildId,
            },
          }),
          prisma.playbackJob.deleteMany({
            where: {
              guildId,
            },
          }),
          prisma.memeBoardItem.deleteMany({
            where: {
              guildId,
            },
          }),
          prisma.guild.deleteMany({
            where: {
              id: guildId,
            },
          }),
        ]);
        const orphanMediaResult = removeOrphanMedia
          ? await purgeOrphanMediaAssets(candidateMediaAssetIds)
          : {
              deletedCount: 0,
              fileDeleteErrors: 0,
            };

        return reply.send({
          purged: true,
          guildId,
          stop: {
            releasedPlayingJobs: stopResult.releasedCount,
            disconnectedOverlaySockets,
          },
          deleted: {
            guildSettings: guildDeleteResult.count,
            overlayClients: overlayDeleteResult.count,
            ingestClients: ingestDeleteCount,
            pairingCodes: pairingDeleteResult.count,
            playbackJobs: playbackDeleteResult.count,
            memeBoardItems: boardDeleteResult.count,
            orphanMediaAssets: orphanMediaResult.deletedCount,
            orphanMediaFileDeleteErrors: orphanMediaResult.fileDeleteErrors,
          },
        });
      },
    );

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

      if (updateResult.count > 0) {
        await disconnectOverlayClient(fastify, clientId);
      }

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
          const isActive = isPairingCodeActive(record, now);
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
          OR: [
            {
              expiresAt: null,
            },
            {
              expiresAt: {
                gt: now,
              },
            },
          ],
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
