/** Discord bot invite, identical to the design mockup. */
export const BOT_INVITE_URL =
  'https://discord.com/oauth2/authorize?client_id=1570564356108714094&scope=bot%20applications.commands&permissions=19456';

/** Slash commands registered by the bot (apps/lc-server/src/discord/registration.rs). */
export const DISCORD_COMMANDS = [
  'dispo',
  'msg',
  'dire',
  'pair-code',
  'help',
  'info',
  'overlays',
  'stop',
  'media-add',
  'lier',
  'cmsg',
  'cdire',
] as const;

/** Fallback used when the runtime settings cannot be read. */
export const MIN_OVERLAY_VERSION_FALLBACK = '3.2.0';

export const LOGIN_ERRORS: Record<string, string> = {
  not_allowed: 'Ce compte Discord n’est pas autorisé à ouvrir la console de ce serveur.',
  oauth_state:
    'La vérification de sécurité Discord a échoué (état OAuth invalide ou expiré). Réessaie la connexion.',
  oauth_failed: 'Discord n’a pas pu confirmer la connexion. Réessaie dans un instant.',
  method_disabled: 'Cette méthode de connexion n’est pas disponible sur ce serveur.',
  unauthorized: 'Ta session a expiré. Reconnecte-toi.',
};
