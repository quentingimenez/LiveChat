interface ProbeMediaHttpStatusParams {
  mediaUrl?: string;
  mediaKind?: string;
  reason?: string;
  jobId?: string;
}

interface AttachMediaDiagnosticsParams {
  mediaUrl?: string;
  mediaKind?: string;
  jobId?: string;
}

interface CreateOverlayLegacyMediaDiagnosticsUtilsOptions {
  toRedactedMediaUrl(value: unknown): string;
  hasTokenInMediaUrl(value: unknown): boolean;
  getHtmlMediaErrorLabel(code: unknown): string;
  logWarning(message: string): void;
}

export interface OverlayLegacyMediaDiagnosticsUtils {
  probeMediaHttpStatus(params: ProbeMediaHttpStatusParams): Promise<void>;
  attachMediaDiagnostics(element: unknown, params: AttachMediaDiagnosticsParams): void;
}

function toSafeString(value: unknown, fallback: string): string {
  if (typeof value !== 'string') {
    return fallback;
  }

  const normalized = value.trim();
  return normalized ? normalized : fallback;
}

function toProbeErrorMessage(error: unknown): string {
  if (typeof error === 'object' && error !== null) {
    const errorRecord = error as Record<string, unknown>;
    if (errorRecord.name === 'AbortError') {
      return 'probe_timeout';
    }

    if (typeof errorRecord.message === 'string' && errorRecord.message.trim()) {
      return errorRecord.message;
    }
  }

  return 'probe_request_failed';
}

export function createOverlayLegacyMediaDiagnosticsUtils(
  options: CreateOverlayLegacyMediaDiagnosticsUtilsOptions
): OverlayLegacyMediaDiagnosticsUtils {
  const { toRedactedMediaUrl, hasTokenInMediaUrl, getHtmlMediaErrorLabel, logWarning } = options;

  async function probeMediaHttpStatus(params: ProbeMediaHttpStatusParams): Promise<void> {
    const mediaUrl = toSafeString(params.mediaUrl, '');
    const mediaKind = toSafeString(params.mediaKind, 'unknown');
    const reason = toSafeString(params.reason, 'unknown');
    const jobId = toSafeString(params.jobId, 'unknown-job');
    const redactedUrl = toRedactedMediaUrl(mediaUrl);

    if (!mediaUrl) {
      return;
    }

    const abortController = typeof AbortController === 'function' ? new AbortController() : null;
    const probeTimeout = setTimeout(() => {
      abortController?.abort();
    }, 6000);

    try {
      const response = await fetch(mediaUrl, {
        method: 'GET',
        headers: {
          Range: 'bytes=0-0'
        },
        cache: 'no-store',
        signal: abortController?.signal
      });
      const statusCode = response.status;
      const statusHint = statusCode === 401 || statusCode === 403 ? 'auth_failed' : 'non_auth_error';

      logWarning(
        `[OVERLAY] Media probe (${reason}) jobId=${jobId} kind=${mediaKind} status=${statusCode} hint=${statusHint} url=${redactedUrl}`
      );

      if (response.body && typeof response.body.cancel === 'function') {
        response.body.cancel().catch(() => undefined);
      }
    } catch (error) {
      const message = toProbeErrorMessage(error);
      logWarning(`[OVERLAY] Media probe failed (${reason}) jobId=${jobId} kind=${mediaKind} error=${message} url=${redactedUrl}`);
    } finally {
      clearTimeout(probeTimeout);
    }
  }

  function attachMediaDiagnostics(element: unknown, params: AttachMediaDiagnosticsParams): void {
    if (!(element instanceof HTMLElement)) {
      return;
    }

    const mediaUrl = toSafeString(params.mediaUrl, '');
    const mediaKind = toSafeString(params.mediaKind, 'unknown');
    const jobId = toSafeString(params.jobId, 'unknown-job');
    const redactedUrl = toRedactedMediaUrl(mediaUrl);
    const tokenPresent = hasTokenInMediaUrl(mediaUrl);
    let hasProbedStatus = false;

    const probeOnce = (reason: string): void => {
      if (hasProbedStatus) {
        return;
      }

      hasProbedStatus = true;
      void probeMediaHttpStatus({
        mediaUrl,
        mediaKind,
        reason,
        jobId
      });
    };

    element.addEventListener('error', () => {
      if (element instanceof HTMLMediaElement) {
        const mediaErrorCode = typeof element.error?.code === 'number' ? element.error.code : 0;
        const mediaError = getHtmlMediaErrorLabel(mediaErrorCode);

        logWarning(
          `[OVERLAY] Media element error jobId=${jobId} kind=${mediaKind} error=${mediaError} tokenPresent=${
            tokenPresent ? 'yes' : 'no'
          } url=${redactedUrl}`
        );
      } else {
        logWarning(
          `[OVERLAY] Image element error jobId=${jobId} kind=${mediaKind} tokenPresent=${tokenPresent ? 'yes' : 'no'} url=${redactedUrl}`
        );
      }

      probeOnce('error');
    });

    if (element instanceof HTMLMediaElement) {
      element.addEventListener('stalled', () => {
        logWarning(
          `[OVERLAY] Media element stalled jobId=${jobId} kind=${mediaKind} readyState=${element.readyState} networkState=${
            element.networkState
          } tokenPresent=${tokenPresent ? 'yes' : 'no'} url=${redactedUrl}`
        );
        probeOnce('stalled');
      });
    }
  }

  return {
    probeMediaHttpStatus,
    attachMediaDiagnostics
  };
}
