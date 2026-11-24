import { isRecord, toNonEmptyString } from './utils';

export type IngestFailureCode =
  | 'SETTINGS_MISSING'
  | 'INVALID_CONFIG'
  | 'NETWORK_ERROR'
  | 'TIMEOUT'
  | 'UNAUTHORIZED'
  | 'INVALID_PAYLOAD'
  | 'MEDIA_INGESTION_FAILED'
  | 'INGEST_DISABLED'
  | 'SERVER_ERROR'
  | 'UNKNOWN';

export interface IngestFailure {
  code: IngestFailureCode;
  message: string;
  status?: number;
  details?: string;
}

/**
 * Corps d'erreur du serveur, réduit aux champs machine.
 *
 * Le champ `message` du serveur n'est volontairement **pas** lu : il ne doit
 * jamais être affiché tel quel (il peut contenir une sortie d'outil ou un
 * détail technique). Les phrases visibles viennent de `MEDIA_FAILURE_MESSAGES`
 * ci-dessous, qui reprend les codes stables du serveur.
 */
interface ParsedIngestErrorBody {
  error?: string;
  code?: string;
}

export const parseIngestErrorBody = (value: unknown): ParsedIngestErrorBody => {
  if (!isRecord(value)) {
    return {};
  }

  return {
    error: toNonEmptyString(value.error) || undefined,
    code: toNonEmptyString(value.code) || undefined
  };
};

/** Phrases locales par code d'ingestion média stable (contrat serveur). */
const MEDIA_FAILURE_MESSAGES: Record<string, string> = {
  UNSUPPORTED_SOURCE: 'Ce lien ou ce format n’est pas pris en charge.',
  PRIVATE_OR_AUTH_REQUIRED: 'Ce média est privé ou demande une connexion.',
  DRM_PROTECTED: 'Ce média est protégé (DRM) et ne peut pas être récupéré.',
  MEDIA_NOT_FOUND: 'Média introuvable à cette adresse.',
  BOARD_STORAGE_LIMIT_REACHED: 'Le board de la guilde est plein.',
  CACHE_STORAGE_LIMIT_REACHED: 'Le stockage du serveur est plein.',
  FILE_TOO_LARGE: 'Fichier trop volumineux pour le serveur.',
  INVALID_MEDIA: 'Ce fichier n’est pas un média lisible.',
  TRANSCODE_FAILED: 'Le serveur n’a pas pu convertir ce média.',
  DOWNLOAD_TIMEOUT: 'Le téléchargement a pris trop de temps.',
  DOWNLOAD_FAILED: 'Le téléchargement du média a échoué.'
};

const DEFAULT_MEDIA_FAILURE_MESSAGE = 'Le média n’a pas pu être ingéré par le serveur.';

const mediaFailureMessage = (legacyCode: string | undefined): string =>
  (legacyCode && MEDIA_FAILURE_MESSAGES[legacyCode]) || DEFAULT_MEDIA_FAILURE_MESSAGE;

export const mapHttpFailure = (status: number, body: unknown): IngestFailure => {
  const parsedBody = parseIngestErrorBody(body);
  const errorLabel = parsedBody.error || '';

  if (status === 400 || errorLabel === 'invalid_payload') {
    return {
      code: 'INVALID_PAYLOAD',
      status,
      message: 'Requête invalide envoyée au serveur LiveChat.'
    };
  }

  if (
    status === 401 ||
    errorLabel === 'unauthorized' ||
    errorLabel === 'ingest_token_expired'
  ) {
    return {
      code: 'UNAUTHORIZED',
      status,
      message:
        errorLabel === 'ingest_token_expired'
          ? 'Ce jeton ingest a expiré. Ré-appairez l’appareil depuis la board LiveChat.'
          : 'Token ingest invalide ou non autorisé.'
    };
  }

  if (status === 422 || errorLabel === 'media_ingestion_failed') {
    return {
      code: 'MEDIA_INGESTION_FAILED',
      status,
      message: mediaFailureMessage(parsedBody.code),
      details: parsedBody.code || undefined
    };
  }

  if (errorLabel === 'guild_mismatch') {
    return {
      code: 'INVALID_CONFIG',
      status,
      message: 'La guilde configurée ne correspond pas au jeton ingest.'
    };
  }

  if (status === 403) {
    return {
      code: 'UNAUTHORIZED',
      status,
      message: 'Le serveur LiveChat a refusé cette action pour ce jeton.'
    };
  }

  if (errorLabel === 'ingest_api_disabled') {
    return {
      code: 'INGEST_DISABLED',
      status,
      message: 'L’endpoint /ingest est désactivé sur ce serveur.'
    };
  }

  if (status === 503 || errorLabel === 'ingest_unavailable') {
    return {
      code: 'SERVER_ERROR',
      status,
      message: 'Le service d’ingestion est momentanément indisponible. Réessayez dans un instant.'
    };
  }

  if (status === 429) {
    return {
      code: 'UNKNOWN',
      status,
      message: 'Trop d’envois simultanés. Réessayez dans un instant.'
    };
  }

  if (status === 504 || errorLabel === 'ingest_timeout') {
    return {
      code: 'SERVER_ERROR',
      status,
      message:
        'Le serveur a interrompu l’envoi : le média n’a pas pu être ingéré à temps. Réessayez avec un lien direct.'
    };
  }

  if (status >= 500) {
    return {
      code: 'SERVER_ERROR',
      status,
      message: 'Le serveur LiveChat a retourné une erreur interne.'
    };
  }

  return {
    code: 'UNKNOWN',
    status,
    message: `Erreur serveur inattendue (${status}).`
  };
};

export const mapNetworkFailure = (error: unknown): IngestFailure => {
  if (error instanceof DOMException && error.name === 'AbortError') {
    return {
      code: 'TIMEOUT',
      message: 'Serveur injoignable (timeout).'
    };
  }

  const errorMessage = error instanceof Error ? error.message : '';
  const normalized = errorMessage.trim().toLowerCase();

  if (normalized.includes('failed to fetch') || normalized.includes('networkerror')) {
    return {
      code: 'NETWORK_ERROR',
      message: 'Impossible de contacter le serveur LiveChat.'
    };
  }

  return {
    code: 'NETWORK_ERROR',
    message: 'Erreur réseau pendant l’envoi vers LiveChat.',
    details: errorMessage || undefined
  };
};
