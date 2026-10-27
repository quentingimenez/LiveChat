import type { IpcMain } from 'electron';
import type { Logger } from '../../shared/types';
import type { OverlayRuntimeConfig } from '../services/config-service';

export interface OverlaySocketLike {
  connected: boolean;
  emit(event: string, payload: unknown): void;
}

export interface PairingHttpResponse {
  statusCode: number;
  body: string;
}

export interface CoreIpcHandlerOptions {
  logger: Logger;
  ipcMain: IpcMain;
  loadConfig: () => OverlayRuntimeConfig;
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
  normalizeServerUrl: (serverUrl: string) => string;
}

export interface OverlayIpcHandlerOptions extends CoreIpcHandlerOptions {
  applyMemeBindings: (
    nextBindings: Record<string, string>,
    options: { strict?: boolean; persist?: boolean }
  ) => {
    ok: boolean;
    appliedBindings: Record<string, string>;
    failedAccelerators: string[];
  };
  emitMemeTriggerSignal: (itemId: unknown, trigger: unknown) => { ok: boolean; reason?: string };
  emitManualStopSignal: () => { ok: boolean; reason?: string };
  sendOverlaySettingsToRenderer: () => void;
  getOverlaySocket: () => OverlaySocketLike | null;
  setPendingPlaybackStatePayload: (payload: unknown) => void;
  setPendingPlaybackStopPayload: (payload: { jobId: string } | null) => void;
}

export interface PairingIpcHandlerOptions extends CoreIpcHandlerOptions {
  httpRequestJson: (
    url: string,
    payload: { code: string; deviceName?: string },
    options?: { rejectUnauthorized?: boolean; timeoutMs?: number }
  ) => Promise<PairingHttpResponse>;
  isLikelyTlsError: (error: unknown) => boolean;
  formatNetworkError: (error: unknown, endpoint: string) => string;
  setGuestMode: (checked: boolean) => OverlayRuntimeConfig;
  sendOverlaySettingsToRenderer: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
  closePairingWindow: () => void;
  updateTrayMenu: () => void;
}

export type RegisterIpcHandlersOptions = OverlayIpcHandlerOptions & PairingIpcHandlerOptions;
