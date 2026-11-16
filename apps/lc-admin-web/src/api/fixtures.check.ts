// Compile-time contract check: every response recorded from the real server
// (src/api/fixtures/*.json, written by lc-server tests_fixtures.rs) must be
// assignable to the type the console declares for it. `npm run typecheck`
// fails when a field the console relies on is missing or has another type.

import type {
  AuditResponse,
  CreateGuildResponse,
  ErrorPayload,
  GuildDetail,
  Health,
  IngestAuthorsResponse,
  IngestionsResponse,
  MediaBoardItemsResponse,
  Overview,
  PairingCodeListResponse,
  RightsResponse,
  RuntimeSettingsResponse,
  Session,
} from './types';

import audit from './fixtures/audit.json';
import createGuild from './fixtures/create-guild.json';
import guildDetail from './fixtures/guild-detail.json';
import guildDetailStandalone from './fixtures/guild-detail-standalone.json';
import health from './fixtures/health.json';
import ingestAuthors from './fixtures/ingest-authors.json';
import ingestions from './fixtures/ingestions.json';
import mediaBoardItems from './fixtures/media-board-items.json';
import overview from './fixtures/overview.json';
import pairingCodes from './fixtures/pairing-codes.json';
import rights from './fixtures/rights.json';
import runtimeSettings from './fixtures/runtime-settings.json';
import session from './fixtures/session.json';
import sessionUnauthorized from './fixtures/session-unauthorized.json';

/**
 * JSON imports type every string as `string`, so string-literal unions of the
 * declared types (`'discord' | 'secret'`) are widened before the comparison.
 * Keys, nullability, numbers, booleans and nesting are still checked.
 */
type Widen<T> = T extends string
  ? string
  : T extends boolean
    ? boolean
    : T extends readonly (infer U)[]
      ? Widen<U>[]
      : T extends object
        ? { [K in keyof T]: Widen<T[K]> }
        : T;

export const recordedResponses = {
  audit: audit satisfies Widen<AuditResponse>,
  createGuild: createGuild satisfies Widen<CreateGuildResponse>,
  guildDetail: guildDetail satisfies Widen<GuildDetail>,
  guildDetailStandalone: guildDetailStandalone satisfies Widen<GuildDetail>,
  health: health satisfies Widen<Health>,
  ingestAuthors: ingestAuthors satisfies Widen<IngestAuthorsResponse>,
  ingestions: ingestions satisfies Widen<IngestionsResponse>,
  mediaBoardItems: mediaBoardItems satisfies Widen<MediaBoardItemsResponse>,
  overview: overview satisfies Widen<Overview>,
  pairingCodes: pairingCodes satisfies Widen<PairingCodeListResponse>,
  rights: rights satisfies Widen<RightsResponse>,
  runtimeSettings: runtimeSettings satisfies Widen<RuntimeSettingsResponse>,
  session: session satisfies Widen<Session>,
  sessionUnauthorized: sessionUnauthorized satisfies Widen<ErrorPayload>,
};
