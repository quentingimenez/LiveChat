import { ApiError, isUnavailable } from './api/client';
import { LOGIN_ERRORS } from './constants';

const CODE_MESSAGES: Record<string, string> = {
  invalid_secret: 'Secret d’administration incorrect.',
  method_disabled: 'Cette méthode de connexion est désactivée sur ce serveur.',
  admin_ui_disabled: 'L’administration est désactivée sur ce serveur.',
  unauthorized: 'Ta session a expiré. Reconnecte-toi.',
  invalid_origin: 'Requête refusée : origine non vérifiée. Recharge la page et réessaie.',
  forbidden: 'Cette action est refusée par les droits actuels.',
  forbidden_role: 'Le rôle actuel ne permet pas cette action.',
  last_admin: 'Impossible de retirer le dernier administrateur de la guilde.',
  guild_not_found: 'Cette guilde n’existe plus.',
  member_not_found: 'Ce membre n’existe plus.',
  overlay_client_not_found: 'Cet overlay n’existe plus.',
  ingest_client_not_found: 'Ce client d’ingestion n’existe plus.',
  pairing_code_not_found: 'Ce code d’appairage n’existe plus.',
  invalid_guild_id: 'Identifiant de guilde invalide.',
  invalid_name: 'Nom de guilde invalide.',
  invalid_label: 'Libellé invalide.',
  invalid_role: 'Rôle invalide.',
  invalid_payload: 'Données envoyées invalides.',
  confirm_guild_id_mismatch: 'L’identifiant saisi ne correspond pas à celui de la guilde.',
  fixed_right: 'Ce droit est figé et ne peut pas être modifié.',
  author_not_found: 'Cet auteur n’est pas connu du serveur.',
  oauth_state: LOGIN_ERRORS.oauth_state,
  oauth_failed: LOGIN_ERRORS.oauth_failed,
  not_allowed: LOGIN_ERRORS.not_allowed,
  network_error: 'Le serveur est injoignable. Vérifie la connexion puis réessaie.',
  invalid_response: 'Le serveur a renvoyé une réponse illisible.',
};

export { isUnavailable };

/** Human-readable French message for any error thrown by the API layer. */
export function describeError(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 0) {
      return CODE_MESSAGES.network_error;
    }
    if (error.status === 404 || error.status === 405) {
      return 'Indisponible sur ce serveur.';
    }
    if (error.status === 401) {
      return CODE_MESSAGES.unauthorized;
    }
    if (error.status === 503) {
      return CODE_MESSAGES.admin_ui_disabled;
    }
    const known = CODE_MESSAGES[error.code];
    if (known) {
      return known;
    }
    if (error.status === 403) {
      return CODE_MESSAGES.forbidden;
    }
    if (error.status === 409) {
      return 'Conflit : l’état du serveur a changé. Recharge puis réessaie.';
    }
    if (error.status >= 500) {
      return 'Erreur interne du serveur. Réessaie dans un instant.';
    }
    return error.message !== error.code ? error.message : 'La requête a échoué.';
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return 'Une erreur inattendue est survenue.';
}

export function loginErrorMessage(code: string | undefined): string | null {
  if (!code) {
    return null;
  }
  return LOGIN_ERRORS[code] ?? `La connexion a échoué (${code}).`;
}

export function confirmUnavailableMessage(): string {
  return 'Indisponible sur ce serveur.';
}
