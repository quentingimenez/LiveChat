import type { OverlayRuntimeConfig } from '../services/config-service';
import type { DisplayLike } from '../services/display-service';
import type { Logger } from '../../shared/types';
import { createRendererSettingsService } from '../services/renderer-settings-service';
import { createOverlaySocketService, type OverlaySocketService } from '../services/socket-service';
import { createAuxiliaryWindowService } from '../windows/auxiliary-window-service';
import { createOverlayWindowService } from '../windows/overlay-window-service';
import { createRuntimeWindowActionsService, type RuntimeWindowActionsService } from './runtime-window-actions-service';

interface OverlaySocketDisconnectOptions {
  nextState?: string;
  reason?: string;
  keepStatus?: boolean;
}

interface CreateRuntimeWindowSocketServiceOptions {
  logger: Logger;
  appIconPath: string;
  preloadPath: string;
  overlayHtmlPath: string;
  pairingHtmlPath: string;
  boardHtmlPath: string;
  getAllDisplays: () => DisplayLike[];
  getAppVersion: () => string;
  loadConfig: () => OverlayRuntimeConfig;
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
  getTargetDisplay: () => DisplayLike;
  getDisplayConfigUpdate: (
    display: DisplayLike,
    index: number
  ) => {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  };
  onTrayMenuNeedsRefresh: () => void;
  onConnectedPeersChange: (peers: unknown) => void;
  onConnectionStateChange: (state: string, reason?: string) => void;
}

export interface RuntimeWindowSocketService {
  runtimeWindowActionsService: RuntimeWindowActionsService;
  overlaySocketService: OverlaySocketService;
  sendOverlaySettingsToRenderer(): void;
  connectOverlaySocket(): void;
  disconnectOverlaySocket(options?: OverlaySocketDisconnectOptions): void;
}

export function createRuntimeWindowSocketService(
  options: CreateRuntimeWindowSocketServiceOptions
): RuntimeWindowSocketService {
  const {
    logger,
    appIconPath,
    preloadPath,
    overlayHtmlPath,
    pairingHtmlPath,
    boardHtmlPath,
    getAllDisplays,
    getAppVersion,
    loadConfig,
    saveConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    normalizeMemeBindings,
    getTargetDisplay,
    getDisplayConfigUpdate,
    onTrayMenuNeedsRefresh,
    onConnectedPeersChange,
    onConnectionStateChange
  } = options;

  const overlayWindowService = createOverlayWindowService({
    appIconPath,
    preloadPath,
    htmlPath: overlayHtmlPath,
    getTargetDisplay,
    getAllDisplays,
    saveDisplaySelection: (display, index) => {
      saveConfig(getDisplayConfigUpdate(display, index));
    },
    onReadyToShow: () => {
      sendOverlaySettingsToRenderer();
    },
    onClosed: () => {
      onTrayMenuNeedsRefresh();
    }
  });

  const auxiliaryWindowService = createAuxiliaryWindowService({
    appIconPath,
    preloadPath,
    pairingHtmlPath,
    boardHtmlPath,
    onBoardReadyToShow: () => {
      sendOverlaySettingsToRenderer();
    }
  });

  const runtimeWindowActionsService = createRuntimeWindowActionsService({
    loadConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    auxiliaryWindowService,
    overlayWindowService
  });

  const rendererSettingsService = createRendererSettingsService({
    loadConfig,
    isGuestModeEnabled,
    normalizeMemeBindings,
    getOverlayWindow: () => runtimeWindowActionsService.getOverlayWindow(),
    getBoardWindow: () => runtimeWindowActionsService.getBoardWindow()
  });

  function sendOverlaySettingsToRenderer(): void {
    rendererSettingsService.sendOverlaySettingsToRenderers();
  }

  const overlaySocketService = createOverlaySocketService({
    logger,
    loadConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    getAppVersion,
    sendOverlayPlay: (payload) => {
      const overlayWindow = runtimeWindowActionsService.getOverlayWindow();
      if (!overlayWindow || overlayWindow.isDestroyed()) {
        return;
      }

      overlayWindow.webContents.send('overlay:play', payload);
    },
    sendOverlayStop: (payload) => {
      const overlayWindow = runtimeWindowActionsService.getOverlayWindow();
      if (!overlayWindow || overlayWindow.isDestroyed()) {
        return;
      }

      overlayWindow.webContents.send('overlay:stop', payload);
    },
    onConnectedPeersChange,
    onConnectionStateChange
  });

  function connectOverlaySocket(): void {
    overlaySocketService.connect();
  }

  function disconnectOverlaySocket(options: OverlaySocketDisconnectOptions = {}): void {
    overlaySocketService.disconnect(options);
  }

  return {
    runtimeWindowActionsService,
    overlaySocketService,
    sendOverlaySettingsToRenderer,
    connectOverlaySocket,
    disconnectOverlaySocket
  };
}
