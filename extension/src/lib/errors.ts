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

interface ParsedIngestErrorBody {
  error?: string;
  code?: string;
  message?: string;
}

const isRecord = (value: unknown): value is Record<string, unknown> => {
  return typeof value === 'object' && value !== null;
};

const asTrimmedString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

export const parseIngestErrorBody = (value: unknown): ParsedIngestErrorBody => {
  if (!isRecord(value)) {
    return {};
  }

  return {
    error: asTrimmedString(value.error) || undefined,
    code: asTrimmedString(value.code) || undefined,
    message: asTrimmedString(value.message) || undefined
  };
};

export const mapHttpFailure = (status: number, body: unknown): IngestFailure => {
  const parsedBody = parseIngestErrorBody(body);
  const errorLabel = parsedBody.error || '';

  if (status === 400 || errorLabel === 'invalid_payload') {
    return {
      code: 'INVALID_PAYLOAD',
      status,
      message: 'Requête invalide envoyée au bot LiveChat.'
    };
  }

  if (status === 401 || errorLabel === 'unauthorized') {
    return {
      code: 'UNAUTHORIZED',
      status,
      message: 'Token ingest invalide ou non autorisé.'
    };
  }

  if (status === 422 || errorLabel === 'media_ingestion_failed') {
    return {
      code: 'MEDIA_INGESTION_FAILED',
      status,
      message: parsedBody.message || 'Le média n’a pas pu être ingéré par le bot.',
      details: parsedBody.code || undefined
    };
  }

  if (status === 503 || errorLabel === 'ingest_api_disabled') {
    return {
      code: 'INGEST_DISABLED',
      status,
      message: 'L’endpoint /ingest est désactivé sur ce bot.'
    };
  }

  if (status >= 500) {
    return {
      code: 'SERVER_ERROR',
      status,
      message: 'Le bot LiveChat a retourné une erreur interne.'
    };
  }

  return {
    code: 'UNKNOWN',
    status,
    message: parsedBody.message || `Erreur serveur inattendue (${status}).`
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
