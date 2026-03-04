import type { Menu, MenuItemConstructorOptions, Tray } from 'electron';
import type { OverlayRuntimeConfig } from './config-service';
import type { DisplayLike } from './display-service';
import { createTrayMenuTemplate } from './tray-menu-service';
import { closeTrayContextMenu, popUpTrayContextMenu } from './tray-context-menu-service';

interface ScreenLike {
  getAllDisplays(): DisplayLike[];
  on(event: 'display-added' | 'display-removed' | 'display-metrics-changed', listener: () => void): void;
}

interface CreateTrayLifecycleServiceOptions {
  appIconPath: string;
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
  onToggleEnabled: (checked: boolean) => void;
  onToggleAutoStart: (checked: boolean) => void;
  onToggleShowText: (checked: boolean) => void;
  onOpenPairing: () => void;
  onOpenBoard: () => void;
  onMoveOverlayToDisplay: (display: DisplayLike, index: number) => void;
  onQuit: () => void;
}

export interface TrayLifecycleService {
  createTray(): void;
  updateTrayMenu(): void;
  showTrayMainMenu(): void;
  closeTrayContextMenu(): void;
}

export function createTrayLifecycleService(options: CreateTrayLifecycleServiceOptions): TrayLifecycleService {
  const {
    appIconPath,
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
    onToggleEnabled,
    onToggleAutoStart,
    onToggleShowText,
    onOpenPairing,
    onOpenBoard,
    onMoveOverlayToDisplay,
    onQuit
  } = options;

  let tray: Tray | null = null;
  let trayMainMenu: Menu | null = null;

  function updateTrayMenu(): void {
    if (!tray) {
      return;
    }

    const config = loadConfig();
    const displays = screen.getAllDisplays();
    const selectedDisplay = getTargetDisplay();
    const selectedDisplayIndex = displays.findIndex((display) => display.id === selectedDisplay.id);
    const autoStartSupported = supportsAutoStart();
    const autoUpdateReason = getAutoUpdateReason();

    tray.setToolTip(buildTrayTooltip(config));

    const template = createTrayMenuTemplate({
      config,
      displays,
      selectedDisplay,
      selectedDisplayIndex,
      autoStartSupported,
      autoUpdateReason,
      autoUpdateStateLabel: getAutoUpdateStateLabel(),
      connectionStateLabel: getConnectionStateLabel(),
      connectionReason: getConnectionReason(),
      appVersion: getAppVersion(),
      canOpenBoard: config.enabled && hasPairingConfig(config) && !isGuestModeEnabled(config),
      buildVolumeMenuItems,
      formatDisplayMenuLabel,
      onToggleEnabled,
      onToggleAutoStart,
      onToggleShowText,
      onOpenPairing,
      onOpenBoard,
      onMoveOverlayToDisplay,
      onQuit
    });

    trayMainMenu = buildMenuFromTemplate(template);
    tray.setContextMenu(null);
  }

  function showTrayMainMenu(): void {
    if (!trayMainMenu) {
      updateTrayMenu();
    }

    if (!tray || !trayMainMenu) {
      return;
    }

    popUpTrayContextMenu(tray, trayMainMenu);
  }

  function closeTrayContextMenuForTray(): void {
    closeTrayContextMenu(tray);
  }

  function createTray(): void {
    tray = createTrayInstance(appIconPath);

    updateTrayMenu();

    tray.on('double-click', () => {
      closeTrayContextMenuForTray();
      onOpenBoard();
    });

    tray.on('right-click', () => {
      closeTrayContextMenuForTray();
      showTrayMainMenu();
    });

    screen.on('display-added', updateTrayMenu);
    screen.on('display-removed', updateTrayMenu);
    screen.on('display-metrics-changed', updateTrayMenu);
  }

  return {
    createTray,
    updateTrayMenu,
    showTrayMainMenu,
    closeTrayContextMenu: closeTrayContextMenuForTray
  };
}
