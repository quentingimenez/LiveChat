/**
 * Live admin data: Server-Sent Events from GET /admin/api/events, with the
 * documented fallback to polling GET /admin/api/overview every 5 seconds when
 * EventSource is unavailable (old browser, proxy that strips SSE, …).
 */

import type { AuditEntry, Ingestion, Overview } from './types';

export interface LiveHandlers {
  onOverview?: (overview: Overview) => void;
  onAudit?: (entry: AuditEntry) => void;
  onIngestion?: (ingestion: Ingestion) => void;
  onPoll?: () => void;
  onFallback?: () => void;
}

const POLL_INTERVAL_MS = 5_000;

function parseEvent<T>(event: MessageEvent<string>): T | null {
  try {
    return JSON.parse(event.data) as T;
  } catch {
    return null;
  }
}

export function subscribeLive(eventsUrl: string, handlers: LiveHandlers): () => void {
  let closed = false;
  let source: EventSource | null = null;
  let pollTimer: number | null = null;

  const stop = () => {
    closed = true;
    source?.close();
    source = null;
    if (pollTimer !== null) {
      window.clearInterval(pollTimer);
      pollTimer = null;
    }
  };

  const startPolling = () => {
    if (closed || pollTimer !== null) {
      return;
    }
    source?.close();
    source = null;
    handlers.onFallback?.();
    pollTimer = window.setInterval(() => {
      handlers.onPoll?.();
    }, POLL_INTERVAL_MS);
  };

  if (typeof EventSource === 'undefined') {
    startPolling();
    return stop;
  }

  try {
    source = new EventSource(eventsUrl, { withCredentials: true });
  } catch {
    startPolling();
    return stop;
  }

  source.addEventListener('overview', (event) => {
    const overview = parseEvent<Overview>(event as MessageEvent<string>);
    if (overview) {
      handlers.onOverview?.(overview);
    }
  });

  source.addEventListener('audit', (event) => {
    const entry = parseEvent<AuditEntry>(event as MessageEvent<string>);
    if (entry) {
      handlers.onAudit?.(entry);
    }
  });

  source.addEventListener('ingestion', (event) => {
    const ingestion = parseEvent<Ingestion>(event as MessageEvent<string>);
    if (ingestion) {
      handlers.onIngestion?.(ingestion);
    }
  });

  source.addEventListener('error', () => {
    // EventSource retries forever; the contract asks for a polling fallback
    // instead, which also surfaces 401s through the normal API client.
    startPolling();
  });

  return stop;
}
