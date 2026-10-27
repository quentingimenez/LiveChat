import { OVERLAY_SOCKET_EVENTS } from '../../shared/socket-events';
import type { Logger } from '../../shared/types';
import type { OverlayRuntimeConfig } from './config-service';

interface OverlaySocketLike {
  on(event: string, listener: (payload: unknown) => void): void;
  io?: {
    on(event: string, listener: () => void): void;
  };
}

interface RegisterOverlaySocketListenersParams {
  overlaySocket: OverlaySocketLike;
  cfg: OverlayRuntimeConfig;
  logger: Logger;
  loadConfig: () => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  sendOverlayPlay: (payload: unknown) => void;
  sendOverlayStop: (payload: unknown) => void;
  onConnectedPeersChange: (peers: unknown) => void;
  onConnectionStateChange: (state: string, reason?: string) => void;
  startHeartbeatLoop: () => void;
  stopHeartbeatLoop: () => void;
  emitHeartbeat: (cfg: OverlayRuntimeConfig) => void;
  flushPendingPayloads: () => void;
}

export function toTrimmedString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

export function getPayloadField(payload: unknown, key: string): string {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    return '';
  }

  return toTrimmedString((payload as Record<string, unknown>)[key]);
}

export function registerOverlaySocketListeners(params: RegisterOverlaySocketListenersParams): void {
  params.overlaySocket.on('connect', () => {
    params.startHeartbeatLoop();
    params.onConnectionStateChange('connected');
    params.emitHeartbeat(params.cfg);
    params.flushPendingPayloads();
  });

  params.overlaySocket.on(OVERLAY_SOCKET_EVENTS.PLAY, (payload) => {
    params.sendOverlayPlay(payload);
  });

  params.overlaySocket.on(OVERLAY_SOCKET_EVENTS.STOP, (payload) => {
    params.sendOverlayStop(payload);
  });

  params.overlaySocket.on(OVERLAY_SOCKET_EVENTS.PEERS, (payload) => {
    const payloadGuildId = getPayloadField(payload, 'guildId');
    const currentGuildId = toTrimmedString(params.loadConfig().guildId);

    if (payloadGuildId && currentGuildId && payloadGuildId !== currentGuildId) {
      return;
    }

    const peers =
      payload && typeof payload === 'object' && !Array.isArray(payload)
        ? (payload as Record<string, unknown>).peers
        : null;

    params.onConnectedPeersChange(peers);
  });

  params.overlaySocket.on('disconnect', (reasonPayload) => {
    const reason = toTrimmedString(reasonPayload);

    params.stopHeartbeatLoop();
    params.onConnectedPeersChange([]);

    const current = params.loadConfig();
    if (!current.enabled) {
      params.onConnectionStateChange('disabled', reason);
      return;
    }

    if (!params.hasPairingConfig(current)) {
      params.onConnectionStateChange('not_paired', reason);
      return;
    }

    if (reason === 'io client disconnect') {
      params.onConnectionStateChange('disconnected', reason);
      return;
    }

    params.onConnectionStateChange('reconnecting', reason);
  });

  params.overlaySocket.on('connect_error', (errorPayload) => {
    params.onConnectedPeersChange([]);
    const message = getPayloadField(errorPayload, 'message') || 'connect_error';
    params.logger.error('Overlay socket connection failed:', message);
    params.onConnectionStateChange('error', message);
  });

  if (params.overlaySocket.io) {
    params.overlaySocket.io.on('reconnect_attempt', () => {
      params.onConnectionStateChange('reconnecting');
    });
  }
}
