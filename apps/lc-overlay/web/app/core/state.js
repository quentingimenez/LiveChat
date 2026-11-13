// Shared application state, labels and permission checks.

export const state = {
  guilds: [],
  activeGuildId: null,
  profile: { displayName: null, avatar: null },
  tab: 'board',
  adminTab: 'members',
  info: null, // GET /overlay/guild response for the active guild
  items: [],
  itemsLoaded: false,
  search: '',
  kind: 'all',
  selectedId: null,
  queue: { playing: null, pending: [] },
  queueStale: false,
  members: [],
  invites: [],
  bindings: {},
  mediaUrls: new Map(),
  capturing: false,
};

export const ROLE_LABELS = { admin: 'Admin', member: 'Membre', guest: 'Invité' };
export const KIND_LABELS = { all: 'Tous', video: 'Vidéo', image: 'Image', audio: 'Audio' };
export const ERROR_LABELS = {
  forbidden: "Ton rôle ne permet pas cette action.",
  read_only_session: 'Les invités ne peuvent pas modifier le board.',
  last_admin: "Tu es le dernier admin : nomme un autre admin d'abord.",
  unauthorized: "Cet appareil n'est plus reconnu par la guilde.",
  pairing_code_invalid_or_expired: 'Code invalide, expiré ou déjà utilisé.',
  pairing_code_scope_mismatch: "Ce code sert à appairer l'extension navigateur, pas un overlay.",
  guild_not_standalone: 'Seule une guilde créée hors Discord peut être liée à un serveur Discord.',
  member_required: 'Action réservée aux membres de la guilde.',
  invalid_role: 'Rôle inconnu.',
  invalid_payload: 'Valeur invalide.',
  invalid_name: 'Nom invalide.',
  board_capacity_exceeded: 'Le board de la guilde est plein.',
  socket_unavailable: 'Guilde déconnectée : impossible de jouer pour le moment.',
};

// Legacy overlay 3.0.0 compatibility: remove in 3.1.
const LEGACY_PERMISSION_KEYS = {
  addMedia: 'addMeme',
  editAnyMedia: 'editAnyMeme',
};

export function can(permission) {
  const permissions = state.info && state.info.permissions;
  if (!permissions) {
    return false;
  }
  if (permissions[permission] !== undefined) {
    return Boolean(permissions[permission]);
  }
  const legacy = LEGACY_PERMISSION_KEYS[permission];
  return Boolean(legacy && permissions[legacy]);
}

export function activeGuild() {
  return state.guilds.find((guild) => guild.membership.guildId === state.activeGuildId) || null;
}

export function guildLabel(guild) {
  return guild.membership.guildName || (state.info && state.info.guild && state.info.guild.name) || 'Guilde';
}

export function describeError(code, fallback) {
  return ERROR_LABELS[code] || fallback || 'Action impossible.';
}
