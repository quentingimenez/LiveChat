import { mapHttpFailure, mapIngestionFailure, mapNetworkFailure, type IngestFailure } from './errors';
import { redactSecrets } from './redact';
import { getSettings, isSettingsComplete, type ExtensionSettings } from './settings';
import { resolveIngestTargetUrl } from './url';
import { isRecord } from './utils';

/** Budget total d'un envoi : soumission puis suivi de l'ingestion. */
const DEFAULT_TIMEOUT_MS = 300000;

/** Intervalle entre deux lectures de `GET /ingest/ingestions/{id}`. */
const DEFAULT_POLL_INTERVAL_MS = 1500;

export interface QuickIngestRequest {
  mode: 'quick';
  url: string;
}

export interface ComposeIngestRequest {
  mode: 'compose';
  url: string;
  text?: string;
  forceRefresh?: boolean;
  saveToBoard?: boolean;
  durationSec?: number;
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
  text?: string;
  forceRefresh?: boolean;
  saveToBoard?: boolean;
  durationSec?: number;
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
  };

  if (request.mode === 'compose') {
    // A save-to-board stores no sent text: only a play keeps a caption.
    if (!request.saveToBoard) {
      const text = toNonEmptyString(request.text);

      if (text) {
        payload.text = text;
      }
    }

    payload.forceRefresh = !!request.forceRefresh;
    payload.saveToBoard = !!request.saveToBoard;

    if (typeof request.durationSec === 'number') {
      payload.durationSec = request.durationSec;
    }
  }

  return payload;
};

/** Succès d'un envoi terminé : job de lecture ou élément de board. */
const readySuccess = (status: number, body: unknown): IngestSuccessResult => {
  const jobId = isRecord(body) ? toNonEmptyString(body.jobId) : null;
  const itemId = isRecord(body) ? toNonEmptyString(body.itemId) : null;

  if (jobId) {
    return { ok: true, status, jobId, message: `Envoyé (job: ${jobId.slice(0, 8)}...)` };
  }

  return {
    ok: true,
    status,
    jobId: null,
    message: itemId ? 'Ajouté au board LiveChat.' : 'Envoyé vers LiveChat.'
  };
};

const sleep = (ms: number): Promise<void> =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });

/**
 * Lit l'état d'une ingestion acceptée jusqu'à sa fin ou jusqu'au délai.
 *
 * Au délai, le média est encore en préparation côté serveur : il sera joué
 * dès qu'il est prêt, l'envoi reste donc un succès.
 */
const waitIngestion = async (
  settings: ExtensionSettings,
  ingestionId: string,
  deadline: number,
  pollIntervalMs: number,
): Promise<IngestResult> => {
  const url = `${settings.apiUrl}/ingest/ingestions/${encodeURIComponent(ingestionId)}`;

  while (Date.now() < deadline) {
    await sleep(Math.min(pollIntervalMs, Math.max(0, deadline - Date.now())));

    const response = await fetch(url, {
      method: 'GET',
      headers: { Authorization: `Bearer ${settings.ingestToken}` },
      signal: AbortSignal.timeout(Math.max(1, deadline - Date.now()))
    });
    const body = await parseResponseBody(response);

    if (!response.ok) {
      if (response.status === 404) {
        return {
          ok: false,
          error: {
            code: 'SERVER_ERROR',
            status: 404,
            message: 'Le serveur LiveChat a perdu le suivi de cet envoi (redémarrage ?). Réessayez.'
          }
        };
      }
      return { ok: false, error: mapHttpFailure(response.status, body) };
    }

    const state = isRecord(body) ? toNonEmptyString(body.status) : null;

    if (state === 'ready') {
      return readySuccess(response.status, body);
    }

    if (state === 'failed') {
      return { ok: false, error: mapIngestionFailure(isRecord(body) ? body.error : null) };
    }
  }

  return {
    ok: true,
    status: 202,
    jobId: null,
    message: 'Envoyé : LiveChat prépare encore ce média, il sera joué dès qu’il est prêt.'
  };
};

export const sendToIngest = async (
  request: IngestRequest,
  providedSettings?: ExtensionSettings | null,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
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

  const deadline = Date.now() + timeoutMs;
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

    // Un média est accepté en `202` puis traité en tâche de fond : le résultat
    // se lit sur `GET /ingest/ingestions/{id}` jusqu'au délai de l'envoi.
    const ingestionId = isRecord(responseBody) ? toNonEmptyString(responseBody.ingestionId) : null;

    if (ingestionId) {
      return await waitIngestion(resolvedSettings, ingestionId, deadline, pollIntervalMs);
    }

    return readySuccess(response.status, responseBody);
  } catch (error) {
    // Le message brut d'une erreur fetch peut inclure l'URL et le header Bearer :
    // on masque le jeton avant de construire le message d'erreur renvoyé.
    const message = error instanceof Error ? error.message : '';
    const safeError = message
      ? new Error(redactSecrets(message, [resolvedSettings.ingestToken]))
      : error;
    return {
      ok: false,
      error: mapNetworkFailure(safeError)
    };
  } finally {
    clearTimeout(timeoutId);
  }
};
