/**
 * Typed client for docs/rust/admin-console-api.md.
 *
 * - Cookies only: every request uses `credentials: 'same-origin'`, and no
 *   token is ever read from or written to web storage.
 * - Mutations send `Content-Type: application/json`.
 * - Each route has one exported function; failures are `ApiError`s carrying
 *   the snake_case `error` code returned by the server.
 */

import type {
  AdminCodeResponse,
  BanMemberResponse,
  AuditResponse,
  AuthMethods,
  ClearMediaBoardResponse,
  ClearQueueResponse,
  CreateDeviceCodeRequest,
  CreateDiscordLinkCodeRequest,
  CreateGuildRequest,
  CreateGuildResponse,
  CreateIngestClientRequest,
  CreateIngestClientResponse,
  CreateInviteRequest,
  CreatedInviteResponse,
  DeleteBoardItemResponse,
  EditMediaBoardItemRequest,
  EditMediaBoardItemResponse,
  DeleteExpiredResponse,
  DeliverDeviceSettingsRequest,
  DeliverDeviceSettingsResponse,
  DiscordLeaveResponse,
  DiscordUnlinkResponse,
  ErrorPayload,
  GuildDetail,
  Health,
  IngestAuthorsResponse,
  IngestionsResponse,
  IssuedCodeResponse,
  MediaBoardItem,
  MediaBoardItemsResponse,
  Overview,
  PairingCodeListResponse,
  PatchMediaBoardItemRequest,
  PatchMediaBoardItemResponse,
  PatchMemberRequest,
  PatchMemberResponse,
  PurgeCacheResponse,
  PurgeGuildResponse,
  RemoveJobResponse,
  RemoveMemberResponse,
  UnbanResponse,
  RevokeClientResponse,
  RevokeClientsResponse,
  RevokeInviteResponse,
  RevokePairingCodeResponse,
  RightsResponse,
  RightsUpdateRequest,
  RuntimeSettingsResponse,
  Session,
  StopGuildResponse,
  UpdateGuildSettingsRequest,
  UpdateGuildSettingsResponse,
  UpdateOverlayClientRequest,
  UpdateOverlayClientResponse,
  CleanOrphansResponse,
  ConfirmGuildRequest,
} from './types';

export const API_BASE = '/admin';

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly payload: ErrorPayload | null;

  constructor(status: number, code: string, message?: string, payload: ErrorPayload | null = null) {
    super(message ?? code);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.payload = payload;
  }
}

type UnauthorizedHandler = () => void;
let unauthorizedHandler: UnauthorizedHandler | null = null;

/** Registered by the app shell: any later 401 goes back to the login screen. */
export function setUnauthorizedHandler(handler: UnauthorizedHandler | null): void {
  unauthorizedHandler = handler;
}

interface RequestOptions {
  /** Set to false for /admin/auth/* calls (a wrong secret is a local error). */
  authRedirect?: boolean;
  signal?: AbortSignal;
}

interface JsonRequestInit {
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  body?: unknown;
  signal?: AbortSignal;
}

function isErrorPayload(value: unknown): value is ErrorPayload {
  return typeof value === 'object' && value !== null && typeof (value as { error?: unknown }).error === 'string';
}

async function request<T>(path: string, init: JsonRequestInit, options: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = { Accept: 'application/json' };
  let body: string | undefined;
  if (init.method !== 'GET') {
    // Every mutation is JSON, including the bodyless ones (revoke/logout).
    headers['Content-Type'] = 'application/json';
  }
  if (init.body !== undefined) {
    body = JSON.stringify(init.body);
  }

  let response: Response;
  try {
    response = await fetch(path, {
      method: init.method,
      headers,
      body,
      credentials: 'same-origin',
      cache: 'no-store',
      signal: init.signal,
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === 'AbortError') {
      throw error;
    }
    throw new ApiError(0, 'network_error', 'Le serveur est injoignable.', null);
  }

  if (response.status === 204 || response.status === 205) {
    return undefined as T;
  }

  const text = await response.text();
  let data: unknown = null;
  if (text) {
    try {
      data = JSON.parse(text) as unknown;
    } catch {
      data = null;
    }
  }

  if (!response.ok) {
    const payload = isErrorPayload(data) ? data : null;
    const code = payload?.error ?? `http_${response.status}`;
    const message = payload?.message ?? code;
    if (response.status === 401 && options.authRedirect !== false) {
      unauthorizedHandler?.();
    }
    throw new ApiError(response.status, code, message, payload);
  }

  if (text && data === null) {
    throw new ApiError(response.status, 'invalid_response', 'Réponse du serveur illisible.', null);
  }

  return data as T;
}

function json<T>(method: JsonRequestInit['method'], path: string, body?: unknown, options?: RequestOptions): Promise<T> {
  return request<T>(path, { method, body }, options);
}

function query(params: Record<string, string | number | boolean | null | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') {
      search.set(key, String(value));
    }
  }
  const suffix = search.toString();
  return suffix ? `?${suffix}` : '';
}

export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError;
}

/** 404/405 means the route is not implemented on this server yet. */
export function isUnavailable(error: unknown): boolean {
  return error instanceof ApiError && (error.status === 404 || error.status === 405);
}

/* --------------------------------------------------------------------- auth */

export function getSession(): Promise<Session> {
  return request<Session>(`${API_BASE}/auth/session`, { method: 'GET' }, { authRedirect: false });
}

/** `POST /admin/auth/secret` — 204 + Set-Cookie, or 401 `invalid_secret`. */
export function loginWithSecret(secret: string): Promise<void> {
  return json<void>('POST', `${API_BASE}/auth/secret`, { secret }, { authRedirect: false });
}

export function logout(): Promise<void> {
  return json<void>('POST', `${API_BASE}/auth/logout`, undefined, { authRedirect: false });
}

/** Plain navigation target, not a fetch: Discord OAuth is a redirect flow. */
export const DISCORD_LOGIN_URL = `${API_BASE}/auth/discord`;

/* --------------------------------------------------------------------- read */

export function getOverview(signal?: AbortSignal): Promise<Overview> {
  return request<Overview>(`${API_BASE}/api/overview`, { method: 'GET', signal });
}

export function getGuild(guildId: string, signal?: AbortSignal): Promise<GuildDetail> {
  return request<GuildDetail>(`${API_BASE}/api/guilds/${encodeURIComponent(guildId)}`, {
    method: 'GET',
    signal,
  });
}

export function getMediaBoardItems(guildId: string, signal?: AbortSignal): Promise<MediaBoardItemsResponse> {
  return request<MediaBoardItemsResponse>(
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/media-board/items`,
    { method: 'GET', signal },
  );
}

export function getHealth(signal?: AbortSignal): Promise<Health> {
  return request<Health>(`${API_BASE}/api/health`, { method: 'GET', signal });
}

export function getIngestions(limit = 50, signal?: AbortSignal): Promise<IngestionsResponse> {
  return request<IngestionsResponse>(`${API_BASE}/api/ingestions${query({ limit })}`, {
    method: 'GET',
    signal,
  });
}

export interface AuditParams {
  query?: string;
  before?: string;
  limit?: number;
}

export function getAudit(params: AuditParams = {}, signal?: AbortSignal): Promise<AuditResponse> {
  return request<AuditResponse>(
    `${API_BASE}/api/audit${query({ query: params.query, before: params.before, limit: params.limit ?? 100 })}`,
    { method: 'GET', signal },
  );
}

/** Direct download link; cookies are sent by the browser. */
export function getAuditExportUrl(params: Pick<AuditParams, 'query'> = {}): string {
  return `${API_BASE}/api/audit/export${query({ query: params.query })}`;
}

export function getRights(signal?: AbortSignal): Promise<RightsResponse> {
  return request<RightsResponse>(`${API_BASE}/api/rights`, { method: 'GET', signal });
}

export function getRuntimeSettings(signal?: AbortSignal): Promise<RuntimeSettingsResponse> {
  return request<RuntimeSettingsResponse>(`${API_BASE}/api/runtime-settings`, { method: 'GET', signal });
}

export interface PairingCodeParams {
  guildId?: string;
  status?: 'all' | 'active' | 'expired';
}

export function getPairingCodes(
  params: PairingCodeParams = {},
  signal?: AbortSignal,
): Promise<PairingCodeListResponse> {
  return request<PairingCodeListResponse>(
    `${API_BASE}/api/pairing-codes${query({ guildId: params.guildId, status: params.status })}`,
    { method: 'GET', signal },
  );
}

export function getIngestAuthors(signal?: AbortSignal): Promise<IngestAuthorsResponse> {
  return request<IngestAuthorsResponse>(`${API_BASE}/api/ingest-authors`, { method: 'GET', signal });
}

/* ---------------------------------------------------------------- mutations */

export function createGuild(body: CreateGuildRequest): Promise<CreateGuildResponse> {
  return json<CreateGuildResponse>('POST', `${API_BASE}/api/guilds`, body);
}

export function createAdminCode(guildId: string): Promise<AdminCodeResponse> {
  return json<AdminCodeResponse>('POST', `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/admin-code`);
}

export function updateGuildSettings(
  guildId: string,
  body: UpdateGuildSettingsRequest,
): Promise<UpdateGuildSettingsResponse> {
  return json<UpdateGuildSettingsResponse>(
    'PATCH',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/settings`,
    body,
  );
}

export function stopGuild(guildId: string): Promise<StopGuildResponse> {
  return json<StopGuildResponse>('POST', `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/stop`);
}

export function revokeGuildClients(guildId: string, confirmGuildId: string): Promise<RevokeClientsResponse> {
  const body: ConfirmGuildRequest = { confirmGuildId };
  return json<RevokeClientsResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/revoke-clients`,
    body,
  );
}

export function clearMediaBoard(guildId: string, confirmGuildId: string): Promise<ClearMediaBoardResponse> {
  const body: ConfirmGuildRequest = { confirmGuildId };
  return json<ClearMediaBoardResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/media-board/clear`,
    body,
  );
}

export function purgeGuild(guildId: string, confirmGuildId: string): Promise<PurgeGuildResponse> {
  const body: ConfirmGuildRequest = { confirmGuildId };
  return json<PurgeGuildResponse>('POST', `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/purge`, body);
}

/** Leaving the Discord server is its own action (`discordLeave`). */
export function discordUnlink(guildId: string, leaveDiscord = false): Promise<DiscordUnlinkResponse> {
  return json<DiscordUnlinkResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/discord-unlink`,
    { leaveDiscord },
  );
}

/** The contract requires confirmGuildId even though the mockup does not ask. */
export function discordLeave(guildId: string): Promise<DiscordLeaveResponse> {
  const body: ConfirmGuildRequest = { confirmGuildId: guildId };
  return json<DiscordLeaveResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/discord-leave`,
    body,
  );
}

export function createDiscordLinkCode(guildId: string): Promise<IssuedCodeResponse> {
  const body: CreateDiscordLinkCodeRequest = {};
  return json<IssuedCodeResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/discord-link-code`,
    body,
  );
}

export function createDeviceCode(guildId: string, body: CreateDeviceCodeRequest): Promise<IssuedCodeResponse> {
  return json<IssuedCodeResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/device-code`,
    body,
  );
}

export function clearQueue(guildId: string): Promise<ClearQueueResponse> {
  return json<ClearQueueResponse>('DELETE', `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/queue`);
}

export function removeQueueJob(guildId: string, jobId: string): Promise<RemoveJobResponse> {
  return json<RemoveJobResponse>(
    'DELETE',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/queue/${encodeURIComponent(jobId)}`,
  );
}

export function patchMember(guildId: string, memberId: string, body: PatchMemberRequest): Promise<PatchMemberResponse> {
  return json<PatchMemberResponse>(
    'PATCH',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/members/${encodeURIComponent(memberId)}`,
    body,
  );
}

export function removeMember(guildId: string, memberId: string): Promise<RemoveMemberResponse> {
  return json<RemoveMemberResponse>(
    'DELETE',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/members/${encodeURIComponent(memberId)}`,
  );
}

export function banMember(guildId: string, memberId: string): Promise<BanMemberResponse> {
  return json<BanMemberResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/members/${encodeURIComponent(memberId)}/ban`,
  );
}

export function unbanMember(guildId: string, banId: string): Promise<UnbanResponse> {
  return json<UnbanResponse>(
    'DELETE',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/bans/${encodeURIComponent(banId)}`,
  );
}

export function createInvite(guildId: string, body: CreateInviteRequest): Promise<CreatedInviteResponse> {
  return json<CreatedInviteResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/invites`,
    body,
  );
}

export function revokeInvite(guildId: string, code: string): Promise<RevokeInviteResponse> {
  return json<RevokeInviteResponse>(
    'POST',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/invites/${encodeURIComponent(code)}/revoke`,
  );
}

export function patchMediaBoardItem(
  guildId: string,
  itemId: string,
  body: PatchMediaBoardItemRequest,
): Promise<PatchMediaBoardItemResponse> {
  return json<PatchMediaBoardItemResponse>(
    'PATCH',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/media-board/items/${encodeURIComponent(itemId)}`,
    body,
  );
}

/** Same route with an edit recipe: answered `202` once the edit is queued. */
export function editMediaBoardItem(
  guildId: string,
  itemId: string,
  body: EditMediaBoardItemRequest,
): Promise<EditMediaBoardItemResponse> {
  return json<EditMediaBoardItemResponse>(
    'PATCH',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/media-board/items/${encodeURIComponent(itemId)}`,
    body,
  );
}

export function deleteMediaBoardItem(guildId: string, itemId: string): Promise<DeleteBoardItemResponse> {
  return json<DeleteBoardItemResponse>(
    'DELETE',
    `${API_BASE}/api/guilds/${encodeURIComponent(guildId)}/media-board/items/${encodeURIComponent(itemId)}`,
  );
}

export function revokeOverlayClient(clientId: string): Promise<RevokeClientResponse> {
  return json<RevokeClientResponse>(
    'POST',
    `${API_BASE}/api/overlay-clients/${encodeURIComponent(clientId)}/revoke`,
  );
}

export function updateOverlayClient(
  clientId: string,
  body: UpdateOverlayClientRequest,
): Promise<UpdateOverlayClientResponse> {
  return json<UpdateOverlayClientResponse>(
    'PATCH',
    `${API_BASE}/api/overlay-clients/${encodeURIComponent(clientId)}`,
    body,
  );
}

export function updateOverlayGuildSettings(
  clientId: string,
  body: DeliverDeviceSettingsRequest,
): Promise<DeliverDeviceSettingsResponse> {
  return json<DeliverDeviceSettingsResponse>(
    'PATCH',
    `${API_BASE}/api/overlay-clients/${encodeURIComponent(clientId)}/guild-settings`,
    body,
  );
}

export function createIngestClient(body: CreateIngestClientRequest): Promise<CreateIngestClientResponse> {
  return json<CreateIngestClientResponse>('POST', `${API_BASE}/api/ingest-clients`, body);
}

export function revokeIngestClient(clientId: string): Promise<RevokeClientResponse> {
  return json<RevokeClientResponse>(
    'POST',
    `${API_BASE}/api/ingest-clients/${encodeURIComponent(clientId)}/revoke`,
  );
}

export function revokePairingCode(code: string): Promise<RevokePairingCodeResponse> {
  return json<RevokePairingCodeResponse>(
    'POST',
    `${API_BASE}/api/pairing-codes/${encodeURIComponent(code)}/revoke`,
  );
}

export function deleteExpiredPairingCodes(guildId?: string): Promise<DeleteExpiredResponse> {
  return json<DeleteExpiredResponse>('DELETE', `${API_BASE}/api/pairing-codes/expired${query({ guildId })}`);
}

export function putRights(body: RightsUpdateRequest): Promise<RightsResponse> {
  return json<RightsResponse>('PUT', `${API_BASE}/api/rights`, body);
}

export function patchRuntimeSettings(body: Record<string, unknown>): Promise<RuntimeSettingsResponse> {
  return json<RuntimeSettingsResponse>('PATCH', `${API_BASE}/api/runtime-settings`, body);
}

export function purgeMediaCache(): Promise<PurgeCacheResponse> {
  return json<PurgeCacheResponse>('POST', `${API_BASE}/api/media/cache/purge`);
}

export function cleanMediaOrphans(): Promise<CleanOrphansResponse> {
  return json<CleanOrphansResponse>('POST', `${API_BASE}/api/media/orphans/clean`);
}

export function getEventsUrl(): string {
  return `${API_BASE}/api/events`;
}

/** Re-exported for screens that build auth error messages. */
export type { AuthMethods };
