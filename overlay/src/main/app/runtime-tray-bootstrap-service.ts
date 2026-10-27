import type { Menu, MenuItemConstructorOptions, Tray } from 'electron';
import type { OverlayRuntimeConfig } from '../services/config-service';
import type { DisplayLike } from '../services/display-service';
import type { RuntimeInteractionService } from './runtime-interaction-service';
import type { RuntimeTrayService } from './runtime-tray-service';
import type { RuntimeWindowActionsService } from './runtime-window-actions-service';

interface ScreenLike {
  getAllDisplays(): DisplayLike[];
  on(event: 'display-added' | 'display-removed' | 'display-metrics-changed', listener: () => void): void;
}

interface InitializeRuntimeTrayBootstrapOptions {
  runtimeTrayService: RuntimeTrayService;
  trayIconPath: string;
  createTrayInstance: (iconPath: string) => Tray;
  buildMenuFromTemplate: (template: MenuItemConstructorOptions[]) => Menu;
  screen: ScreenLike;
  loadConfig: () => OverlayRuntimeConfig;
  getTargetDisplay: () => DisplayLike;
  formatDisplayMenuLabel: (display: DisplayLike, index: number) => string;
  supportsAutoStart: () => boolean;
  getAutoUpdateReason: () => string;
  getAutoUpdateStateLabel: () => string;
  getConnectionStateLabel: () => string;
  getConnectionReason: () => string;
  getAppVersion: () => string;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  buildTrayTooltip: (config?: OverlayRuntimeConfig) => string;
  buildVolumeMenuItems: (currentVolume: number) => MenuItemConstructorOptions[];
  runtimeInteractionService: RuntimeInteractionService;
  runtimeWindowActionsService: RuntimeWindowActionsService;
  disconnectOverlaySocket: () => void;
  quitApp: () => void;
}

export function initializeRuntimeTrayBootstrap(options: InitializeRuntimeTrayBootstrapOptions): void {
  const {
    runtimeTrayService,
    trayIconPath,
    createTrayInstance,
    buildMenuFromTemplate,
    screen,
    loadConfig,
    getTargetDisplay,
    formatDisplayMenuLabel,
    supportsAutoStart,
    getAutoUpdateReason,
    getAutoUpdateStateLabel,
    getConnectionStateLabel,
    getConnectionReason,
    getAppVersion,
    hasPairingConfig,
    isGuestModeEnabled,
    buildTrayTooltip,
    buildVolumeMenuItems,
    runtimeInteractionService,
    runtimeWindowActionsService,
    disconnectOverlaySocket,
    quitApp
  } = options;

  runtimeTrayService.initialize({
    trayIconPath,
    createTrayInstance,
    buildMenuFromTemplate,
    screen,
    loadConfig,
    getTargetDisplay,
    formatDisplayMenuLabel,
    supportsAutoStart,
    getAutoUpdateReason,
    getAutoUpdateStateLabel,
    getConnectionStateLabel,
    getConnectionReason,
    getAppVersion,
    hasPairingConfig,
    isGuestModeEnabled,
    buildTrayTooltip,
    buildVolumeMenuItems,
    onToggleEnabled: (checked) => {
      void runtimeInteractionService.setEnabledAsync(checked);
    },
    onToggleAutoStart: (checked) => {
      runtimeInteractionService.toggleAutoStart(checked);
    },
    onToggleShowText: (checked) => {
      runtimeInteractionService.toggleShowText(checked);
    },
    onOpenPairing: () => {
      runtimeWindowActionsService.createPairingWindow();
    },
    onOpenBoard: () => {
      runtimeWindowActionsService.createBoardWindow();
    },
    onMoveOverlayToDisplay: (display, index) => {
      runtimeInteractionService.moveOverlayToDisplay(display, index);
    },
    onQuit: () => {
      disconnectOverlaySocket();
      runtimeWindowActionsService.destroyOverlayWindow();
      runtimeWindowActionsService.destroyBoardWindow();
      runtimeWindowActionsService.closePairingWindow();
      quitApp();
    }
  });
}
