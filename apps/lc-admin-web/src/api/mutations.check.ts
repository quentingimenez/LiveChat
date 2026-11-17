// Mutation responses recorded by the server (tests_fixtures.rs,
// `admin_console_mutation_fixtures`), checked against the types the console
// reads them with. A missing field or a wrong type fails `npm run typecheck`.

import type {
  AdminCodeResponse,
  BanMemberResponse,
  CleanOrphansResponse,
  ClearMediaBoardResponse,
  ClearQueueResponse,
  CreateIngestClientResponse,
  CreatedInviteResponse,
  DeleteBoardItemResponse,
  DeleteExpiredResponse,
  DeliverDeviceSettingsResponse,
  DiscordLeaveResponse,
  DiscordUnlinkResponse,
  EditMediaBoardItemResponse,
  IssuedCodeResponse,
  PatchMediaBoardItemResponse,
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
  StopGuildResponse,
  UpdateGuildSettingsResponse,
  UpdateOverlayClientResponse,
} from './types';

import mutationAdminCode from './fixtures/mutation-admin-code.json';
import mutationCleanMediaOrphans from './fixtures/mutation-clean-media-orphans.json';
import mutationClearMediaBoard from './fixtures/mutation-clear-media-board.json';
import mutationClearQueue from './fixtures/mutation-clear-queue.json';
import mutationCreateIngestClient from './fixtures/mutation-create-ingest-client.json';
import mutationCreateInvite from './fixtures/mutation-create-invite.json';
import mutationDeleteBoardItem from './fixtures/mutation-delete-board-item.json';
import mutationDeleteExpiredPairingCodes from './fixtures/mutation-delete-expired-pairing-codes.json';
import mutationDeviceCode from './fixtures/mutation-device-code.json';
import mutationDiscordLeave from './fixtures/mutation-discord-leave.json';
import mutationDiscordLinkCode from './fixtures/mutation-discord-link-code.json';
import mutationEditBoardItem from './fixtures/mutation-edit-board-item.json';
import mutationDiscordUnlink from './fixtures/mutation-discord-unlink.json';
import mutationOverlayGuildSettings from './fixtures/mutation-overlay-guild-settings.json';
import mutationPatchBoardItem from './fixtures/mutation-patch-board-item.json';
import mutationPatchMember from './fixtures/mutation-patch-member.json';
import mutationPurgeGuild from './fixtures/mutation-purge-guild.json';
import mutationPurgeMediaCache from './fixtures/mutation-purge-media-cache.json';
import mutationRemoveMember from './fixtures/mutation-remove-member.json';
import mutationBanMember from './fixtures/mutation-ban-member.json';
import mutationUnban from './fixtures/mutation-unban.json';
import mutationRemoveQueueJob from './fixtures/mutation-remove-queue-job.json';
import mutationRevokeGuildClients from './fixtures/mutation-revoke-guild-clients.json';
import mutationRevokeIngestClient from './fixtures/mutation-revoke-ingest-client.json';
import mutationRevokeInvite from './fixtures/mutation-revoke-invite.json';
import mutationRevokeOverlayClient from './fixtures/mutation-revoke-overlay-client.json';
import mutationRevokePairingCode from './fixtures/mutation-revoke-pairing-code.json';
import mutationStopGuild from './fixtures/mutation-stop-guild.json';
import mutationUpdateGuildSettings from './fixtures/mutation-update-guild-settings.json';
import mutationUpdateOverlayClient from './fixtures/mutation-update-overlay-client.json';

type Widen<T> = T extends string
  ? string
  : T extends boolean
    ? boolean
    : T extends readonly (infer U)[]
      ? Widen<U>[]
      : T extends object
        ? { [K in keyof T]: Widen<T[K]> }
        : T;

export const recordedMutationResponses = {
  adminCode: mutationAdminCode satisfies Widen<AdminCodeResponse>,
  cleanMediaOrphans: mutationCleanMediaOrphans satisfies Widen<CleanOrphansResponse>,
  clearMediaBoard: mutationClearMediaBoard satisfies Widen<ClearMediaBoardResponse>,
  clearQueue: mutationClearQueue satisfies Widen<ClearQueueResponse>,
  createIngestClient: mutationCreateIngestClient satisfies Widen<CreateIngestClientResponse>,
  createInvite: mutationCreateInvite satisfies Widen<CreatedInviteResponse>,
  deleteBoardItem: mutationDeleteBoardItem satisfies Widen<DeleteBoardItemResponse>,
  deleteExpiredPairingCodes: mutationDeleteExpiredPairingCodes satisfies Widen<DeleteExpiredResponse>,
  deviceCode: mutationDeviceCode satisfies Widen<IssuedCodeResponse>,
  discordLeave: mutationDiscordLeave satisfies Widen<DiscordLeaveResponse>,
  discordLinkCode: mutationDiscordLinkCode satisfies Widen<IssuedCodeResponse>,
  editBoardItem: mutationEditBoardItem satisfies Widen<EditMediaBoardItemResponse>,
  discordUnlink: mutationDiscordUnlink satisfies Widen<DiscordUnlinkResponse>,
  overlayGuildSettings: mutationOverlayGuildSettings satisfies Widen<DeliverDeviceSettingsResponse>,
  patchBoardItem: mutationPatchBoardItem satisfies Widen<PatchMediaBoardItemResponse>,
  patchMember: mutationPatchMember satisfies Widen<PatchMemberResponse>,
  purgeGuild: mutationPurgeGuild satisfies Widen<PurgeGuildResponse>,
  purgeMediaCache: mutationPurgeMediaCache satisfies Widen<PurgeCacheResponse>,
  removeMember: mutationRemoveMember satisfies Widen<RemoveMemberResponse>,
  banMember: mutationBanMember satisfies Widen<BanMemberResponse>,
  unbanMember: mutationUnban satisfies Widen<UnbanResponse>,
  removeQueueJob: mutationRemoveQueueJob satisfies Widen<RemoveJobResponse>,
  revokeGuildClients: mutationRevokeGuildClients satisfies Widen<RevokeClientsResponse>,
  revokeIngestClient: mutationRevokeIngestClient satisfies Widen<RevokeClientResponse>,
  revokeInvite: mutationRevokeInvite satisfies Widen<RevokeInviteResponse>,
  revokeOverlayClient: mutationRevokeOverlayClient satisfies Widen<RevokeClientResponse>,
  revokePairingCode: mutationRevokePairingCode satisfies Widen<RevokePairingCodeResponse>,
  stopGuild: mutationStopGuild satisfies Widen<StopGuildResponse>,
  updateGuildSettings: mutationUpdateGuildSettings satisfies Widen<UpdateGuildSettingsResponse>,
  updateOverlayClient: mutationUpdateOverlayClient satisfies Widen<UpdateOverlayClientResponse>,
};
