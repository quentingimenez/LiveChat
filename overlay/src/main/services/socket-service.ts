import { io } from 'socket.io-client';
import { OVERLAY_SOCKET_EVENTS } from '../../shared/socket-events';
import type { Logger } from '../../shared/types';
import type { OverlayRuntimeConfig } from './config-service';
import { getPayloadField, registerOverlaySocketListeners } from './socket-listeners-service';

interface OverlaySocketLike {
  connected: boolean;
  disconnect(): void;
  emit(event: string, payload: unknown): void;
  on(event: string, listener: (payload: unknown) => void): void;
  io?: {
    on(event: string, listener: () => void): void;
  };
}

interface DisconnectOptions {
  nextState?: string;
  reason?: string;
  keepStatus?: boolean;
}

interface CreateOverlaySocketServiceOptions {
  logger: Logger;
  loadConfig: () => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  getAppVersion: () => string;
  sendOverlayPlay: (payload: unknown) => void;
  sendOverlayStop: (payload: unknown) => void;
  onConnectedPeersChange: (peers: unknown) => void;
  onConnectionStateChange: (state: string, reason?: string) => void;
}

export interface OverlaySocketService {
  connect(): void;
  disconnect(options?: DisconnectOptions): void;
  stopHeartbeatLoop(): void;
  getSocket(): OverlaySocketLike | null;
  setPendingPlaybackStatePayload(payload: unknown): void;
  setPendingPlaybackStopPayload(payload: { jobId: string } | null): void;
  clearPendingPlayback(): void;
}

export function createOverlaySocketService(options: CreateOverlaySocketServiceOptions): OverlaySocketService {
  const {
    logger,
    loadConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    getAppVersion,
    sendOverlayPlay,
    sendOverlayStop,
    onConnectedPeersChange,
    onConnectionStateChange
  } = options;

  let overlaySocket: OverlaySocketLike | null = null;
  let heartbeatInterval: ReturnType<typeof setInterval> | null = null;
  let pendingPlaybackStatePayload: unknown = null;
  let pendingPlaybackStopPayload: { jobId: string } | null = null;

  function emitHeartbeat(cfg: OverlayRuntimeConfig): void {
    if (!overlaySocket || !overlaySocket.connected) {
      return;
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.HEARTBEAT, {
      clientId: cfg.clientId || 'unknown-client',
      guildId: cfg.guildId || 'unknown-guild',
      appVersion: getAppVersion(),
      sessionMode: isGuestModeEnabled(cfg) ? 'invite_read_only' : 'normal'
    });
  }

  function stopHeartbeatLoop(): void {
    if (heartbeatInterval) {
      clearInterval(heartbeatInterval);
      heartbeatInterval = null;
    }
  }

  function startHeartbeatLoop(): void {
    stopHeartbeatLoop();

    heartbeatInterval = setInterval(() => {
      emitHeartbeat(loadConfig());
    }, 15000);
  }

  function disconnect({ nextState = 'disconnected', reason = '', keepStatus = false }: DisconnectOptions = {}): void {
    stopHeartbeatLoop();
    onConnectedPeersChange([]);

    if (overlaySocket) {
      overlaySocket.disconnect();
      overlaySocket = null;
    }

    if (!keepStatus) {
      onConnectionStateChange(nextState, reason);
    }
  }

  function flushPendingPayloads(): void {
    if (!overlaySocket || !overlaySocket.connected) {
      return;
    }

    if (pendingPlaybackStatePayload) {
      overlaySocket.emit(OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE, pendingPlaybackStatePayload);
      const jobId = getPayloadField(pendingPlaybackStatePayload, 'jobId') || 'unknown';
      const state = getPayloadField(pendingPlaybackStatePayload, 'state') || 'unknown';
      logger.info(`[OVERLAY] Flushed buffered playback-state (jobId: ${jobId}, state: ${state})`);

      pendingPlaybackStatePayload = null;
    }

    if (pendingPlaybackStopPayload) {
      overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, pendingPlaybackStopPayload);
      logger.info(`[OVERLAY] Flushed buffered playback-stop (jobId: ${pendingPlaybackStopPayload.jobId || 'unknown'})`);
      pendingPlaybackStopPayload = null;
    }
  }

  function connect(): void {
    const cfg = loadConfig();

    disconnect({ keepStatus: true });

    if (!cfg.enabled || !hasPairingConfig(cfg)) {
      onConnectionStateChange(cfg.enabled ? 'not_paired' : 'disabled');
      return;
    }

    onConnectionStateChange('connecting');

    overlaySocket = io(cfg.serverUrl || '', {
      transports: ['websocket'],
      reconnection: true,
      reconnectionDelay: 1000,
      reconnectionDelayMax: 3000,
      timeout: 10000,
      rejectUnauthorized: false,
      auth: {
        token: cfg.clientToken,
        sessionMode: isGuestModeEnabled(cfg) ? 'invite_read_only' : 'normal'
      }
    }) as OverlaySocketLike;

    registerOverlaySocketListeners({
      overlaySocket,
      cfg,
      logger,
      loadConfig,
      hasPairingConfig,
      sendOverlayPlay,
      sendOverlayStop,
      onConnectedPeersChange,
      onConnectionStateChange,
      startHeartbeatLoop,
      stopHeartbeatLoop,
      emitHeartbeat,
      flushPendingPayloads,
    });
  }

  return {
    connect,
    disconnect,
    stopHeartbeatLoop,
    getSocket(): OverlaySocketLike | null {
      return overlaySocket;
    },
    setPendingPlaybackStatePayload(payload: unknown): void {
      pendingPlaybackStatePayload = payload;
    },
    setPendingPlaybackStopPayload(payload: { jobId: string } | null): void {
      pendingPlaybackStopPayload = payload;
    },
    clearPendingPlayback(): void {
      pendingPlaybackStatePayload = null;
      pendingPlaybackStopPayload = null;
    }
  };
}
