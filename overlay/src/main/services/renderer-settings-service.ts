import { IPC_CHANNELS } from '../../shared/ipc';
import type { OverlaySettingsPayload } from '../../shared/overlay-config';
import type { Dictionary } from '../../shared/types';
import type { OverlayRuntimeConfig } from './config-service';

interface WebContentsLike {
  send(channel: string, payload: unknown): void;
}

interface RendererWindowLike {
  isDestroyed(): boolean;
  webContents: WebContentsLike;
}

interface CreateRendererSettingsServiceOptions {
  loadConfig: () => OverlayRuntimeConfig;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  normalizeMemeBindings: (candidate: unknown) => Dictionary<string>;
  getOverlayWindow: () => RendererWindowLike | null;
  getBoardWindow: () => RendererWindowLike | null;
}

export interface RendererSettingsService {
  buildOverlaySettingsPayload(config?: OverlayRuntimeConfig): OverlaySettingsPayload;
  sendOverlaySettingsToRenderers(): void;
}

function sendToWindow(window: RendererWindowLike | null, payload: OverlaySettingsPayload): void {
  if (!window || window.isDestroyed()) {
    return;
  }

  window.webContents.send(IPC_CHANNELS.overlaySettings, payload);
}

export function createRendererSettingsService(options: CreateRendererSettingsServiceOptions): RendererSettingsService {
  const { loadConfig, isGuestModeEnabled, normalizeMemeBindings, getOverlayWindow, getBoardWindow } = options;

  function buildOverlaySettingsPayload(config: OverlayRuntimeConfig = loadConfig()): OverlaySettingsPayload {
    return {
      serverUrl: config.serverUrl,
      clientToken: config.clientToken,
      guildId: config.guildId,
      clientId: config.clientId,
      guestMode: isGuestModeEnabled(config),
      memeBindings: normalizeMemeBindings(config.memeBindings),
      volume: config.volume,
      showText: config.showText
    };
  }

  function sendOverlaySettingsToRenderers(): void {
    const payload = buildOverlaySettingsPayload();
    sendToWindow(getOverlayWindow(), payload);
    sendToWindow(getBoardWindow(), payload);
  }

  return {
    buildOverlaySettingsPayload,
    sendOverlaySettingsToRenderers
  };
}
