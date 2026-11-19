/**
 * Types of the admin console API contract (docs/rust/admin-console-api.md, v2).
 *
 * Dates are ISO 8601 UTC strings with milliseconds, sizes are bytes, and JSON
 * keys are camelCase. Fields that only exist on the guild-detail route are
 * marked as optional on the overview entry so a stale server can still be
 * rendered without crashing.
 */

export type IsoDate = string;

export interface AuthMethods {
  discord: boolean;
  secret: boolean;
}

export interface SessionUser {
  id: string;
  name: string;
  avatarUrl: string | null;
}

export interface Session {
  method: 'discord' | 'secret';
  user: SessionUser | null;
  expiresAt: IsoDate;
}

export interface ErrorPayload {
  error: string;
  message?: string;
  methods?: AuthMethods;
  [key: string]: unknown;
}

export type DiscordKind = 'native' | 'linked' | 'none';
export type GuildRole = 'guest' | 'member' | 'admin';
export type OverlaySessionMode = 'normal' | 'invite_read_only';
export type OverlayPlatform = 'windows' | 'macos' | 'linux';

export interface GuildSettings {
  defaultMediaTime: number;
  maxMediaTime: number | null;
  displayMediaFull: boolean;
  overrides?: {
    defaultMediaTime: number | null;
    maxMediaTime: number | null;
    displayMediaFull: boolean | null;
  };
}

export interface DiscordLink {
  kind: DiscordKind;
  serverId: string | null;
  botPresent?: boolean | null;
}

export interface OverlayGuildSettings {
  volume: number;
  mediaMuted: boolean;
  paused: boolean;
}

export interface OverlayClient {
  id: string;
  label: string;
  authorName: string | null;
  authorImage: string | null;
  lastSeenAt: IsoDate | null;
  createdAt: IsoDate;
  connected: boolean;
  sessionMode: OverlaySessionMode;
  /* Guild-detail enrichment (absent from the overview payload). */
  guildId?: string;
  memberId?: string | null;
  role?: GuildRole | null;
  overlayVersion?: string | null;
  platform?: OverlayPlatform | string | null;
  guildSettings?: OverlayGuildSettings | null;
}

export interface IngestClient {
  id: string;
  label: string;
  lastSeenAt: IsoDate | null;
  createdAt: IsoDate;
  /* Optional enrichment used when the server exposes it. */
  guildId?: string;
  defaultAuthorName?: string | null;
  defaultAuthorImage?: string | null;
}

export interface MediaBoardStats {
  itemsCount: number;
  usedBytes: number;
  maxBytes: number;
  remainingBytes: number;
  usedPercent: number;
}

export interface PlaybackStats {
  pending: number;
  playing: number;
  failed: number;
  done: number;
}

export interface GuildEntry {
  id: string;
  name: string;
  editableName: string;
  discord: DiscordLink;
  busyUntil: IsoDate | null;
  settings: GuildSettings;
  overlays: {
    total: number;
    connectedCount: number;
    clients: OverlayClient[];
  };
  ingest: {
    total: number;
    clients: IngestClient[];
  };
  mediaBoard: MediaBoardStats;
  playback: PlaybackStats;
}

export interface GuildMember {
  id: string;
  displayName: string;
  avatar: string | null;
  role: GuildRole;
  createdAt: IsoDate;
  discordLinked: boolean;
  profileSource: 'discord' | 'custom' | string;
  connected?: boolean;
  deviceCount?: number;
  lastSeenAt?: IsoDate | null;
  isSelf?: boolean;
  discordUserId?: string | null;
  discordAdmin?: boolean;
  discordName?: string | null;
}

export interface GuildInvite {
  code: string;
  role: GuildRole;
  memberId: string | null;
  maxUses: number | null;
  useCount: number;
  createdByMemberId: string | null;
  expiresAt: IsoDate | null;
  revokedAt: IsoDate | null;
  createdAt: IsoDate;
  isActive: boolean;
}

export interface GuildBan {
  id: string;
  memberId: string | null;
  displayName: string;
  avatar: string | null;
  discordUserId: string | null;
  bannedBy: string | null;
  createdAt: IsoDate;
}

export interface QueueJob {
  id: string;
  status: 'playing' | 'pending' | string;
  title: string | null;
  text: string | null;
  authorName: string | null;
  authorImage: string | null;
  durationSec: number;
  mediaAssetId: string | null;
  kind: 'video' | 'audio' | 'image' | string | null;
  submittedAt: IsoDate | null;
  startedAt: IsoDate | null;
}

export interface QueueState {
  playing: QueueJob | null;
  pending: QueueJob[];
  serverTime?: IsoDate | null;
}

export interface GuildDetail extends GuildEntry {
  guestsCanViewBoard: boolean;
  boardQuotaBytes: number | null;
  discord: DiscordLink & { botPresent: boolean | null };
  members: {
    count: number;
    items: GuildMember[];
  };
  invites: {
    count: number;
    items: GuildInvite[];
  };
  bans: {
    count: number;
    items: GuildBan[];
  };
  queue: QueueState;
}

export interface OverviewCache {
  usedBytes: number;
  maxBytes: number;
  remainingBytes: number;
  usedPercent: number;
}

export interface Overview {
  generatedAt: IsoDate;
  cache: OverviewCache;
  totals: {
    guildCount: number;
    overlayClientCount: number;
    overlayConnectedCount: number;
    ingestClientCount: number;
    pairingCodeCount: number;
    /** Invitations usable now, all guilds together. */
    activeInviteCount: number;
  };
  pairingCodes: {
    active: number;
    expired: number;
    total: number;
  };
  guilds: GuildEntry[];
}

export interface HealthCheck {
  id: 'database' | 'discord' | 'ffmpeg' | 'ytdlp' | 'piper' | 'ingestions' | string;
  status: 'ok' | 'warn' | 'error' | string;
  detail: string;
  version: string | null;
  /** Number behind the check (failed ingestions in the last hour). */
  count: number | null;
}

export interface Health {
  status: 'ok' | 'degraded' | string;
  checks: HealthCheck[];
  uptimeSec: number;
  serverVersion: string;
}

export interface Ingestion {
  id: string;
  guildId: string;
  guildName: string;
  source: string;
  url: string;
  status: 'pending' | 'running' | 'done' | 'failed' | string;
  error: string | null;
  startedAt: IsoDate;
  finishedAt: IsoDate | null;
  durationMs: number | null;
}

export interface IngestionsResponse {
  items: Ingestion[];
}

export interface AuditActor {
  method: string;
  name: string;
}

export interface AuditEntry {
  id: number;
  at: IsoDate;
  actor: AuditActor;
  action: string;
  method: string;
  path: string;
  status: number;
  guildId: string | null;
  clientIp: string | null;
}

export interface AuditResponse {
  items: AuditEntry[];
  nextBefore: string | null;
}

export interface PairingCode {
  code: string;
  guildId: string;
  createdByDiscordUserId: string | null;
  authorName: string | null;
  expiresAt: IsoDate | null;
  usedAt: IsoDate | null;
  createdAt: IsoDate;
  isActive: boolean;
}

export interface PairingCodeListResponse {
  items: PairingCode[];
  counts: {
    total: number;
    active: number;
    expired: number;
  };
}

export interface IngestAuthor {
  discordUserId: string;
  displayName: string;
  image: string | null;
  lastSeenAt: IsoDate | null;
  source: 'pairing' | 'ingest' | 'mixed' | string;
}

export interface IngestAuthorsResponse {
  items: IngestAuthor[];
  count: number;
}

export interface CreatedIngestClient {
  id: string;
  guildId: string;
  label: string;
  defaultAuthorName: string;
  defaultAuthorImage: string | null;
  createdByDiscordUserId: string;
  createdAt: IsoDate;
}

export interface CreateIngestClientResponse {
  created: true;
  ingestApiToken: string;
  client: CreatedIngestClient;
}

export interface AdminInvite {
  code: string;
  role: GuildRole;
  memberId: string | null;
  maxUses: number | null;
  useCount: number;
  createdByMemberId: string | null;
  expiresAt: IsoDate | null;
  revokedAt: IsoDate | null;
  createdAt: IsoDate;
  isActive: boolean;
}

export interface CreateGuildResponse {
  created: true;
  guild: {
    id: string;
    name: string;
  };
  adminInvite: AdminInvite;
}

export interface AdminCodeResponse {
  created: true;
  guildId: string;
  adminInvite: AdminInvite;
}

export interface UpdateGuildSettingsResponse {
  updated: true;
  guild: {
    id: string;
    defaultMediaTime: number | null;
    maxMediaTime: number | null;
    displayMediaFull: boolean;
    busyUntil: IsoDate | null;
    name?: string;
    guestsCanViewBoard?: boolean;
    boardQuotaMb?: number | null;
  };
}

export interface StopGuildResponse {
  stopped: true;
  guildId: string;
  releasedCount: number;
}

export interface RevokeClientsResponse {
  guildId: string;
  stop: {
    releasedPlayingJobs: number;
    disconnectedOverlaySockets: number;
  };
  revoked: {
    overlayClients: number;
    ingestClients: number;
    members: number;
    pairingCodes: number;
    invites: number;
  };
}

export interface ClearMediaBoardResponse {
  guildId: string;
  deleted: {
    mediaBoardItems: number;
    orphanMediaAssets: number;
    orphanMediaFileDeleteErrors: number;
  };
}

export interface DiscordDeparture {
  serverId: string | null;
  requested: boolean;
  outcome: string;
  error: string | null;
}

export interface PurgeGuildResponse {
  purged: true;
  guildId: string;
  stop: {
    releasedPlayingJobs: number;
    disconnectedOverlaySockets: number;
  };
  deleted: Record<string, number>;
  discord: DiscordDeparture;
}

export interface DiscordUnlinkResponse {
  unlinked: true;
  guildId: string;
  discord: DiscordDeparture;
}

export interface DiscordLeaveResponse {
  discordLeave: DiscordDeparture;
}

export interface RevokePairingCodeResponse {
  found: boolean;
  revoked: boolean;
  guildId: string;
}

export interface DeleteExpiredResponse {
  deletedCount: number;
  guildId: string | null;
}

export interface RevokeClientResponse {
  found: boolean;
  revoked: boolean;
  clientId: string;
  guildId: string;
  label: string;
}

export interface RemoveJobResponse {
  removed: boolean;
}

export interface ClearQueueResponse {
  removedCount: number;
}

export interface PatchMemberResponse {
  updated: true;
  member: {
    id: string;
    role: GuildRole;
  };
}

export interface RemoveMemberResponse {
  removed: true;
}

export interface BanMemberResponse {
  banned: true;
  ban: GuildBan;
}

export interface UnbanResponse {
  unbanned: true;
}

export interface CreatedInviteResponse {
  created: true;
  invite: GuildInvite;
}

export interface DeleteBoardItemResponse {
  deleted: boolean;
}

export interface RevokeInviteResponse {
  revoked: boolean;
}

/** Device / Discord link codes have a deliberately tolerant shape. */
export interface IssuedCodeResponse {
  code?: string;
  command?: string;
  expiresAt?: string;
  [key: string]: unknown;
}

export interface UpdateOverlayClientResponse extends OverlayClient {
  updated?: true;
}

export interface DeliverDeviceSettingsResponse {
  delivered: boolean;
}

export interface MediaAssetSummary {
  id: string;
  kind: 'video' | 'audio' | 'image' | string;
  mime: string;
  /** `null` for images and texts. */
  durationSec: number | null;
  width: number | null;
  height: number | null;
  isVertical: boolean | null;
  sizeBytes: number;
  sourceUrl: string;
}

export interface MediaBoardItem {
  id: string;
  guildId: string;
  mediaAssetId: string;
  title: string;
  message: string | null;
  createdByName: string;
  /* Stable author identity (`member:<uuid>`, `discord:<id>` or `name:<nickname>`);
     a member and their linked Discord account share one id. */
  createdByAuthorId?: string | null;
  createdAt: IsoDate;
  /* Display time chosen for an image item, in seconds (null: guild default). */
  displayDurationSec: number | null;
  /* Scale chosen for an image item, in percent (null: natural size). */
  displayScalePercent: number | null;
  media: MediaAssetSummary;
  /* Favorite of the caller; the console is not a member, so always false. */
  favorite: boolean;
}

export interface MediaBoardItemsResponse {
  items: MediaBoardItem[];
  nextOffset?: number;
}

export interface RightsCell {
  allowed: boolean;
  fixed: boolean;
  default: boolean;
}

export interface RightsAction {
  id: string;
  cells: Record<string, RightsCell>;
}

export interface RightsSurface {
  id: 'overlay' | 'extension' | 'discord' | string;
  columns: string[];
  actions: RightsAction[];
}

export interface RightsResponse {
  surfaces: RightsSurface[];
  discordAdminIsAdmin: boolean;
}

export interface RightsUpdateCell {
  surface: string;
  action: string;
  role: string;
  allowed: boolean;
}

export interface RightsUpdateRequest {
  cells: RightsUpdateCell[];
  discordAdminIsAdmin?: boolean;
}

export interface RuntimeSettingEntry {
  value: unknown;
  source: 'runtime' | 'env' | 'default' | string;
  default?: unknown;
}

export interface RuntimeSettingsResponse {
  /* v2 payload. */
  settings?: Record<string, RuntimeSettingEntry>;
  readOnly?: Record<string, unknown>;
  /* Legacy payload still returned by the current server. */
  tiktokCookie?: string;
  hasTikTokCookie?: boolean;
}

export interface PurgeCacheResponse {
  removedFiles: number;
  freedBytes: number;
}

export interface CleanOrphansResponse {
  removedFiles: number;
  freedBytes: number;
}

/* ---------------------------------------------------------------- requests */

export interface CreateGuildRequest {
  name: string;
}

export interface UpdateGuildSettingsRequest {
  name?: string;
  defaultMediaTime?: number | null;
  maxMediaTime?: number | null;
  displayMediaFull?: boolean;
  guestsCanViewBoard?: boolean;
  boardQuotaMb?: number | null;
}

export interface CreateInviteRequest {
  role: GuildRole;
  expiresInHours?: number | null;
  maxUses?: number | null;
}

export interface PatchMemberRequest {
  role: GuildRole;
}

export interface CreateDeviceCodeRequest {
  memberId: string;
}

export interface CreateDiscordLinkCodeRequest {
  [key: string]: never;
}

export interface UpdateOverlayClientRequest {
  label?: string;
  sessionMode?: OverlaySessionMode;
}

export interface DeliverDeviceSettingsRequest {
  volume?: number;
  mediaMuted?: boolean;
  paused?: boolean;
}

export interface PatchMediaBoardItemRequest {
  title?: string;
  message?: string | null;
  artworkOnOverlay?: boolean;
  displayScalePercent?: number | null;
}

export interface PatchMediaBoardItemResponse {
  updated: boolean;
  item: {
    id: string;
    title: string;
    message: string | null;
    artworkOnOverlay: boolean;
    displayDurationSec: number | null;
    displayScalePercent: number | null;
  };
}

/** Subset of `lc_media::edit::EditRecipe` the console uses (cut only). */
export interface EditMediaBoardItemRequest {
  replace: true;
  recipe: { segments: [number, number][] };
}

export interface EditMediaBoardItemResponse {
  ingestionId: string;
}

export interface CreateIngestClientRequest {
  guildId: string;
  authorDiscordUserId: string;
  label: string;
}

export interface ConfirmGuildRequest {
  confirmGuildId: string;
}
