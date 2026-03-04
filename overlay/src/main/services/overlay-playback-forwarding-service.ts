import { isRecord, toTrimmedString } from '../../shared/guards';
import { OVERLAY_SOCKET_EVENTS } from '../../shared/socket-events';
import type { Logger } from '../../shared/types';

interface OverlaySocketEmitter {
  connected: boolean;
  emit(event: string, payload: unknown): void;
}

interface CreateOverlayPlaybackForwardingServiceOptions {
  logger: Logger;
  isGuestModeEnabled: () => boolean;
  getOverlaySocket: () => OverlaySocketEmitter | null;
  setPendingPlaybackStatePayload: (payload: unknown) => void;
  setPendingPlaybackStopPayload: (payload: { jobId: string } | null) => void;
}

export interface OverlayPlaybackForwardingService {
  handleOverlayError(payload: unknown): void;
  handleOverlayPlaybackState(payload: unknown): void;
  handleOverlayPlaybackStop(payload: unknown): void;
}

export function normalizePlaybackStopPayload(payload: unknown): { jobId: string } {
  const payloadRecord = isRecord(payload) ? payload : null;
  const normalizedJobId = toTrimmedString(payloadRecord?.jobId);

  return {
    jobId: normalizedJobId || 'unknown'
  };
}

export function createOverlayPlaybackForwardingService(
  options: CreateOverlayPlaybackForwardingServiceOptions
): OverlayPlaybackForwardingService {
  const { logger, isGuestModeEnabled, getOverlaySocket, setPendingPlaybackStatePayload, setPendingPlaybackStopPayload } = options;

  function handleOverlayError(payload: unknown): void {
    if (isGuestModeEnabled()) {
      return;
    }

    const overlaySocket = getOverlaySocket();
    if (!overlaySocket || !overlaySocket.connected) {
      return;
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.ERROR, payload);
  }

  function handleOverlayPlaybackState(payload: unknown): void {
    if (isGuestModeEnabled()) {
      return;
    }

    const overlaySocket = getOverlaySocket();
    if (!overlaySocket || !overlaySocket.connected) {
      setPendingPlaybackStatePayload(payload || null);

      const payloadRecord = isRecord(payload) ? payload : {};
      const jobId = toTrimmedString(payloadRecord.jobId) || 'unknown';
      const state = toTrimmedString(payloadRecord.state) || 'unknown';

      logger.warn(`[OVERLAY] Buffered playback-state while socket offline (jobId: ${jobId}, state: ${state})`);
      return;
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE, payload);

    const payloadRecord = isRecord(payload) ? payload : {};
    const jobId = toTrimmedString(payloadRecord.jobId) || 'unknown';
    const state = toTrimmedString(payloadRecord.state) || 'unknown';
    const remainingMs = typeof payloadRecord.remainingMs === 'number' ? payloadRecord.remainingMs : 'null';

    logger.info(
      `[OVERLAY] Forwarded playback-state to bot (jobId: ${jobId}, state: ${state}, remainingMs: ${remainingMs})`
    );
  }

  function handleOverlayPlaybackStop(payload: unknown): void {
    if (isGuestModeEnabled()) {
      return;
    }

    const normalizedPayload = normalizePlaybackStopPayload(payload);
    const overlaySocket = getOverlaySocket();

    if (!overlaySocket || !overlaySocket.connected) {
      setPendingPlaybackStopPayload(normalizedPayload);
      logger.warn(`[OVERLAY] Buffered playback-stop while socket offline (jobId: ${normalizedPayload.jobId})`);
      return;
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, normalizedPayload);
    logger.info(`[OVERLAY] Forwarded playback-stop to bot (jobId: ${normalizedPayload.jobId})`);
  }

  return {
    handleOverlayError,
    handleOverlayPlaybackState,
    handleOverlayPlaybackStop
  };
}
