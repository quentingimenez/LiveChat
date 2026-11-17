import type {
  AuditEntry,
  GuildDetail,
  GuildEntry,
  Health,
  Overview,
  RightsResponse,
} from './api/types';

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;

export const guildFixture: GuildEntry = {
  id: '812345678901234567',
  name: 'Les Zigotos du Dimanche',
  editableName: 'Les Zigotos du Dimanche',
  discord: { kind: 'native', serverId: '812345678901234567', botPresent: true },
  busyUntil: null,
  settings: {
    defaultMediaTime: 6,
    maxMediaTime: 45,
    displayMediaFull: false,
    overrides: { defaultMediaTime: null, maxMediaTime: null, displayMediaFull: null },
  },
  overlays: {
    total: 2,
    connectedCount: 1,
    clients: [
      {
        id: 'client-1',
        label: 'PC-Salon-Quentin',
        authorName: 'Quentin',
        authorImage: null,
        lastSeenAt: null,
        createdAt: '2026-10-01T10:00:00.000Z',
        connected: true,
        sessionMode: 'normal',
      },
      {
        id: 'client-2',
        label: 'Portable-Yanis',
        authorName: 'Yanis',
        authorImage: null,
        lastSeenAt: '2026-11-14T09:00:00.000Z',
        createdAt: '2026-10-02T10:00:00.000Z',
        connected: false,
        sessionMode: 'invite_read_only',
      },
    ],
  },
  ingest: {
    total: 1,
    clients: [
      {
        id: 'ingest-1',
        label: 'Chrome-Quentin',
        lastSeenAt: null,
        createdAt: '2026-10-03T10:00:00.000Z',
      },
    ],
  },
  mediaBoard: {
    itemsCount: 2,
    usedBytes: 5 * GIB,
    maxBytes: 15 * GIB,
    remainingBytes: 10 * GIB,
    usedPercent: 33.33,
  },
  playback: { pending: 0, playing: 1, failed: 0, done: 42 },
};

export const overviewFixture: Overview = {
  generatedAt: '2026-11-15T12:00:00.000Z',
  cache: {
    usedBytes: 512 * MIB,
    maxBytes: 5 * GIB,
    remainingBytes: 5 * GIB - 512 * MIB,
    usedPercent: 10,
  },
  totals: {
    guildCount: 1,
    overlayClientCount: 2,
    overlayConnectedCount: 1,
    ingestClientCount: 1,
    pairingCodeCount: 1,
    activeInviteCount: 2,
  },
  pairingCodes: { active: 1, expired: 0, total: 1 },
  guilds: [guildFixture],
};

export const guildDetailFixture: GuildDetail = {
  ...guildFixture,
  guestsCanViewBoard: true,
  boardQuotaBytes: null,
  discord: { kind: 'native', serverId: '812345678901234567', botPresent: true },
  members: {
    count: 2,
    items: [
      {
        id: 'member-1',
        displayName: 'Quentin',
        avatar: null,
        role: 'admin',
        createdAt: '2026-01-01T10:00:00.000Z',
        discordLinked: true,
        profileSource: 'discord',
        connected: true,
        deviceCount: 1,
        lastSeenAt: null,
        discordUserId: '111111111111111111',
        discordAdmin: true,
      },
      {
        id: 'member-2',
        displayName: 'Yanis',
        avatar: null,
        role: 'guest',
        createdAt: '2026-02-01T10:00:00.000Z',
        discordLinked: false,
        profileSource: 'custom',
        connected: false,
        deviceCount: 1,
        lastSeenAt: '2026-11-14T09:00:00.000Z',
      },
    ],
  },
  bans: {
    count: 1,
    items: [
      {
        id: 'ban-1',
        memberId: 'member-9',
        displayName: 'Troll',
        avatar: null,
        discordUserId: '999999999999999999',
        bannedBy: 'Quentin',
        createdAt: '2026-11-10T10:00:00.000Z',
      },
    ],
  },
  invites: {
    count: 1,
    items: [
      {
        code: 'ABCD2345',
        role: 'member',
        memberId: null,
        maxUses: 1,
        useCount: 0,
        createdByMemberId: null,
        expiresAt: '2026-11-16T12:00:00.000Z',
        revokedAt: null,
        createdAt: '2026-11-15T12:00:00.000Z',
        isActive: true,
      },
    ],
  },
  queue: {
    playing: {
      id: 'job-1',
      status: 'playing',
      title: 'Le chat qui tombe du canapé',
      text: null,
      authorName: 'Quentin',
      authorImage: null,
      durationSec: 14,
      mediaAssetId: 'asset-1',
      kind: 'video',
      submittedAt: '2026-11-15T11:59:30.000Z',
      startedAt: '2026-11-15T11:59:45.000Z',
    },
    pending: [],
    serverTime: '2026-11-15T12:00:00.000Z',
  },
  overlays: {
    total: 2,
    connectedCount: 1,
    clients: guildFixture.overlays.clients.map((client, index) =>
      index === 0
        ? {
            ...client,
            memberId: 'member-1',
            role: 'admin' as const,
            overlayVersion: '3.4.0',
            platform: 'windows',
            guildSettings: { volume: 80, mediaMuted: false, paused: false },
          }
        : {
            ...client,
            memberId: 'member-2',
            role: 'guest' as const,
            overlayVersion: '3.1.2',
            platform: 'windows',
            guildSettings: { volume: 100, mediaMuted: false, paused: false },
          },
    ),
  },
};

export const healthFixture: Health = {
  status: 'ok',
  checks: [
    { id: 'database', status: 'ok', detail: 'database reachable', version: null, count: null },
    { id: 'discord', status: 'ok', detail: 'bot connected', version: null, count: null },
    { id: 'ffmpeg', status: 'ok', detail: 'ffmpeg version 7.1', version: '7.1', count: null },
  ],
  uptimeSec: 6 * 86400 + 4 * 3600,
  serverVersion: '3.4.0',
};

export const auditFixture: AuditEntry[] = [
  {
    id: 1,
    at: '2026-11-15T11:58:00.000Z',
    actor: { method: 'secret', name: 'quentin' },
    action: 'Overlay révoqué',
    method: 'POST',
    path: '/admin/api/overlay-clients/client-2/revoke',
    status: 200,
    guildId: guildFixture.id,
    clientIp: '127.0.0.1',
  },
];

export const rightsFixture: RightsResponse = {
  discordAdminIsAdmin: true,
  surfaces: [
    {
      id: 'overlay',
      columns: ['guest', 'member', 'admin'],
      actions: [
        {
          id: 'receiveMedia',
          cells: {
            guest: { allowed: true, fixed: true, default: true },
            member: { allowed: true, fixed: true, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
        {
          id: 'viewQueue',
          cells: {
            guest: { allowed: false, fixed: false, default: false },
            member: { allowed: true, fixed: false, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
        {
          id: 'viewBoard',
          cells: {
            guest: { allowed: false, fixed: false, default: false },
            member: { allowed: true, fixed: false, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
        {
          id: 'addMedia',
          cells: {
            guest: { allowed: false, fixed: false, default: false },
            member: { allowed: true, fixed: false, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
      ],
    },
    {
      id: 'extension',
      columns: ['guest', 'member', 'admin'],
      actions: [
        {
          id: 'pair',
          cells: {
            guest: { allowed: false, fixed: false, default: false },
            member: { allowed: true, fixed: false, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
        {
          id: 'send',
          cells: {
            guest: { allowed: false, fixed: false, default: false },
            member: { allowed: true, fixed: false, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
      ],
    },
    {
      id: 'discord',
      columns: ['none', 'guest', 'member', 'admin'],
      actions: [
        {
          id: 'info',
          cells: {
            none: { allowed: true, fixed: true, default: true },
            guest: { allowed: true, fixed: true, default: true },
            member: { allowed: true, fixed: true, default: true },
            admin: { allowed: true, fixed: true, default: true },
          },
        },
      ],
    },
  ],
};
