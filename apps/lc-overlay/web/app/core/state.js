// Shared application state, labels and permission checks.

export const state = {
  guilds: [],
  activeGuildId: null,
  profile: { displayName: null, avatar: null },
  // 'guild' shows the active guild (board, queue, admin); 'settings' is the
  // device-wide page, available with zero guild.
  page: 'guild',
  tab: 'board',
  adminTab: 'members',
  info: null, // GET /overlay/guild response for the active guild
  // `true` once the guild info was read (or restored from the cache): the
  // header can then trust `permissions`.
  infoLoaded: false,
  infoError: null,
  // `true` while the active guild changed and its data is still being read:
  // the rail and the header already show the new guild, the view shows its
  // cached snapshot or a skeleton.
  switchingGuild: false,
  pendingGuildId: null,
  items: [],
  itemsLoaded: false,
  itemsError: null,
  // Shape only (kind + width/height) of the last board visit, restored from
  // the persistent layout cache while the real items are unknown.
  boardLayout: null,

  search: '',
  kind: 'all',
  // Board filters and sort (see core/board-filters.js).
  filters: { favorites: false, shortcut: false, author: '', source: '', duration: '', sort: 'newest' },
  filtersOpen: false,
  selectedId: null,
  // `true` once the detail drawer was opened by a click (it stays open);
  // `false` while it only follows the pointer.
  detailPinned: false,
  queue: { playing: null, pending: [], at: 0 },
  queueStale: false,
  queueLoaded: false,
  queueError: null,
  queueHistory: [],
  uploads: [],
  triggers: new Map(), // jobId -> itemId, for the playback badges
  triggerHints: new Map(), // jobId -> { outcome, position, at } until the queue is read again
  members: [],
  devices: [],
  invites: [],
  pairingCodes: [],
  bans: [],
  auditEntries: [],
  bindings: {},
  capturing: false,
  // Selection mode of the board (see views/board.js): picked item ids and the
  // last picked one, the start of a Shift+click range.
  selecting: false,
  selection: new Set(),
  selectionAnchor: null,
};

export const ROLE_LABELS = { admin: 'Admin', member: 'Membre', guest: 'Invité' };
export const KIND_LABELS = { all: 'Tous', video: 'Vidéo', image: 'Image', audio: 'Audio' };
export const ERROR_LABELS = {
  forbidden: "Ton rôle ne permet pas cette action.",
  read_only_session: 'Les invités ne peuvent pas modifier le board.',
  last_admin: "Tu es le dernier admin : nomme un autre admin d'abord.",
  unauthorized: "Cet appareil n'est plus reconnu par la guilde.",
  pairing_code_invalid_or_expired: 'Code invalide, expiré ou déjà utilisé.',
  display_name_required: 'Choisis un pseudo : ce code d’invitation ne vient pas de Discord.',
  pairing_code_scope_mismatch: "Ce code sert à appairer l'extension navigateur, pas un overlay.",
  guild_not_standalone: 'Seule une guilde créée hors Discord peut être liée à un serveur Discord.',
  member_required: 'Action réservée aux membres de la guilde.',
  discord_already_linked: 'Ce compte Discord est déjà lié à un autre membre de la guilde.',
  member_banned: 'Ce compte Discord est banni de la guilde.',
  cannot_ban_self: 'Tu ne peux pas te bannir toi-même.',
  cannot_revoke_self: 'Tu ne peux pas révoquer cet appareil : c’est celui que tu utilises.',
  device_not_found: 'Cet appareil n’existe plus.',
  ban_not_found: 'Ce bannissement a déjà été levé.',
  invalid_role: 'Rôle inconnu.',
  invalid_payload: 'Valeur invalide.',
  invalid_name: 'Nom invalide.',
  invalid_board_quota: 'Quota trop élevé : il ne peut pas dépasser la limite du serveur.',
  confirmation_mismatch: 'Le nom saisi ne correspond pas à celui de la guilde.',
  invalid_limit: 'Nombre d’entrées invalide.',
  board_capacity_exceeded: 'Le board de la guilde est plein.',
  socket_unavailable: 'Guilde déconnectée : impossible de jouer pour le moment.',
  ack_timeout: 'Le serveur n’a pas confirmé la lecture.',
  trigger_refused: 'Le serveur a refusé la lecture.',
  read_only: 'Les invités ne peuvent pas lancer de média.',
  item_not_found: 'Ce média n’est plus sur le board.',
  media_not_ready: 'Ce média n’est pas encore prêt. Réessaie dans un instant.',
  rate_limited: 'Trop de lectures d’affilée : attends un peu.',
  internal_error: 'Le serveur n’a pas pu lancer ce média.',
  payload_too_large: 'Fichier trop volumineux pour le serveur.',
  unsupported_media: 'Format de fichier non pris en charge.',
  timeout: 'Le serveur met trop de temps à répondre.',
  settings_conflict: 'Ces réglages ont été modifiés sur un autre appareil. Recharge-les avant d’enregistrer.',
};

export function can(permission) {
  const permissions = state.info && state.info.permissions;
  return Boolean(permissions && permissions[permission]);
}

// Whether the active guild opens the « Administration » tab.
export function canAdmin() {
  return can('manageMembers') || can('manageInvites') || can('manageSettings');
}

export function activeGuild() {
  return state.guilds.find((guild) => guild.membership.guildId === state.activeGuildId) || null;
}

export function guildLabel(guild) {
  // The live `/overlay/guild` name wins for the active guild: another board can
  // rename it, and the cached membership keeps the name read at invite time.
  const liveName =
    guild.membership.guildId === state.activeGuildId && state.info && state.info.guild
      ? state.info.guild.name
      : null;
  return liveName || guild.membership.guildName || 'Guilde';
}

export function describeError(code, fallback) {
  return ERROR_LABELS[code] || fallback || 'Action impossible.';
}
