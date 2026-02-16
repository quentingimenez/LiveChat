import { mapHttpFailure, mapNetworkFailure, type IngestFailure } from './errors';
import { DEFAULT_AUTHOR_NAME, getSettings, isSettingsComplete, type ExtensionSettings } from './settings';
import { resolveIngestTargetUrl } from './url';

const DEFAULT_TIMEOUT_MS = 300000;

export interface QuickIngestRequest {
  mode: 'quick';
  url: string;
}

export interface ComposeIngestRequest {
  mode: 'compose';
  url: string;
  text?: string;
  forceRefresh?: boolean;
}

export type IngestRequest = QuickIngestRequest | ComposeIngestRequest;

export interface IngestSuccessResult {
  ok: true;
  jobId: string | null;
  status: number;
  message: string;
}

export interface IngestFailureResult {
  ok: false;
  error: IngestFailure;
}

export type IngestResult = IngestSuccessResult | IngestFailureResult;

type IngestPayload = {
  guildId: string;
  url: string;
  authorName: string;
  authorImage?: string;
  text?: string;
  forceRefresh?: boolean;
};

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const parseResponseBody = async (response: Response): Promise<unknown> => {
  const contentType = response.headers.get('content-type') || '';

  if (contentType.includes('application/json')) {
    try {
      return await response.json();
    } catch {
      return null;
    }
  }

  try {
    const textPayload = await response.text();
    const normalized = textPayload.trim();

    if (!normalized) {
      return null;
    }

    return {
      message: normalized
    };
  } catch {
    return null;
  }
};

export const buildIngestPayload = (request: IngestRequest, settings: ExtensionSettings): IngestPayload | null => {
  const normalizedUrl = resolveIngestTargetUrl(request.url);

  if (!normalizedUrl) {
    return null;
  }

  const payload: IngestPayload = {
    guildId: settings.guildId,
    url: normalizedUrl,
    authorName: settings.authorName || DEFAULT_AUTHOR_NAME
  };

  if (settings.authorImage) {
    payload.authorImage = settings.authorImage;
  }

  if (request.mode === 'compose') {
    const text = toNonEmptyString(request.text);

    if (text) {
      payload.text = text;
    }

    payload.forceRefresh = !!request.forceRefresh;
  }

  return payload;
};

export const sendToIngest = async (
  request: IngestRequest,
  providedSettings?: ExtensionSettings | null,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<IngestResult> => {
  const resolvedSettings = providedSettings ?? (await getSettings());

  if (!isSettingsComplete(resolvedSettings)) {
    return {
      ok: false,
      error: {
        code: 'SETTINGS_MISSING',
        message: 'Configuration extension incomplète. Ouvre les options LiveChat.'
      }
    };
  }

  const payload = buildIngestPayload(request, resolvedSettings);

  if (!payload) {
    return {
      ok: false,
      error: {
        code: 'INVALID_PAYLOAD',
        message: 'URL invalide ou non supportée pour l’envoi.'
      }
    };
  }

  const controller = new AbortController();
  const timeoutId = setTimeout(() => {
    controller.abort();
  }, timeoutMs);

  try {
    const response = await fetch(`${resolvedSettings.apiUrl}/ingest`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${resolvedSettings.ingestToken}`
      },
      body: JSON.stringify(payload),
      signal: controller.signal
    });

    const responseBody = await parseResponseBody(response);

    if (!response.ok) {
      return {
        ok: false,
        error: mapHttpFailure(response.status, responseBody)
      };
    }

    const jobId =
      responseBody && typeof responseBody === 'object' && responseBody !== null && 'jobId' in responseBody
        ? toNonEmptyString((responseBody as { jobId?: unknown }).jobId) || null
        : null;

    return {
      ok: true,
      status: response.status,
      jobId,
      message: jobId ? `Envoyé (job: ${jobId.slice(0, 8)}...)` : 'Envoyé vers LiveChat.'
    };
  } catch (error) {
    return {
      ok: false,
      error: mapNetworkFailure(error)
    };
  } finally {
    clearTimeout(timeoutId);
  }
};
