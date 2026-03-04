import { app, globalShortcut, ipcMain, Menu, nativeImage, screen, Tray } from 'electron';
import { httpRequestJson, isLikelyTlsError, formatNetworkError } from '../infra/http-client';
import { logger } from '../infra/logger';
import { applyRuntimeProcessGuards } from './runtime-process-guard-service';
import { bootstrapRuntimeService } from './runtime-bootstrap-service';
import { createRuntimeCoreServices } from './runtime-core-services-factory';
import { createRuntimeInteractionService, type RuntimeInteractionService } from './runtime-interaction-service';
import { resolveRuntimePaths } from './runtime-paths';
import { initializeRuntimeTrayBootstrap } from './runtime-tray-bootstrap-service';
import { createRuntimeTrayService } from './runtime-tray-service';
import { createRuntimeWindowSocketService } from './runtime-window-socket-service';

const WINDOWS_APP_USER_MODEL_ID = 'com.livechat';
const LEGACY_WINDOWS_APP_USER_MODEL_ID = 'com.overlay.client';
const MANUAL_RELOAD_SHORTCUT = 'Shift+Escape';
const MAX_OTHER_ACTIVE_OVERLAYS_IN_TOOLTIP = 2;
const STARTUP_AUTO_UPDATE_TIMEOUT_MS = 30000;

export function runLegacyRuntime(): void {
  const hasSingleInstanceLock = applyRuntimeProcessGuards({
    app,
    platform: process.platform,
    windowsAppUserModelId: WINDOWS_APP_USER_MODEL_ID,
    ignoreCertificateErrorsSwitch: 'ignore-certificate-errors',
    autoplayPolicySwitch: 'autoplay-policy',
    autoplayPolicyValue: 'no-user-gesture-required'
  });

  let isQuittingForUpdate = false;
  const runtimeTrayService = createRuntimeTrayService();
  const updateTrayMenu = (): void => runtimeTrayService.updateTrayMenu();
  const createTray = (): void => runtimeTrayService.createTray();
  let runtimeInteractionService: RuntimeInteractionService | null = null;

  const runtimePaths = resolveRuntimePaths({
    userDataPath: app.getPath('userData'),
    baseDir: __dirname
  });
  const { configPath, appIconPath, preloadScriptPath, overlayHtmlPath, pairingHtmlPath, boardHtmlPath } = runtimePaths;

  const { runtimeCoreFacadeService, autoStartRuntimeService, autoUpdateService } = createRuntimeCoreServices({
    logger,
    app,
    platform: process.platform,
    processExecPath: process.execPath,
    portableExecutableFileRaw: process.env.PORTABLE_EXECUTABLE_FILE,
    disableAutoUpdateByEnvRawValue: process.env.LIVECHAT_DISABLE_AUTO_UPDATE,
    windowsAppUserModelId: WINDOWS_APP_USER_MODEL_ID,
    legacyWindowsAppUserModelId: LEGACY_WINDOWS_APP_USER_MODEL_ID,
    configPath,
    maxOtherActiveOverlaysInTooltip: MAX_OTHER_ACTIVE_OVERLAYS_IN_TOOLTIP,
    startupAutoUpdateTimeoutMs: STARTUP_AUTO_UPDATE_TIMEOUT_MS,
    getAllDisplays: () => screen.getAllDisplays(),
    getPrimaryDisplay: () => screen.getPrimaryDisplay(),
    onTrayMenuNeedsRefresh: updateTrayMenu,
    onStartupInstallRequested: () => {
      isQuittingForUpdate = true;
    },
    onVolumeSelected: (volume) => {
      runtimeInteractionService?.changeVolume(volume);
    }
  });

  const {
    normalizeVolume,
    normalizeGuestMode,
    normalizeMemeBindings,
    loadConfig,
    saveConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    getTargetDisplay,
    getDisplayConfigUpdate,
    formatDisplayMenuLabel,
    getConnectionStateLabel,
    setOverlayConnectionState,
    getOverlayConnectionReason,
    setConnectedOverlayPeers,
    buildTrayTooltip,
    buildVolumeMenuItems
  } = runtimeCoreFacadeService;

  const getAutoUpdateStateLabel = (): string => autoUpdateService.getStateLabel();

  const runtimeWindowSocketService = createRuntimeWindowSocketService({
    logger,
    appIconPath,
    preloadPath: preloadScriptPath,
    overlayHtmlPath,
    pairingHtmlPath,
    boardHtmlPath,
    getAllDisplays: () => screen.getAllDisplays(),
    getAppVersion: () => app.getVersion(),
    loadConfig,
    saveConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    normalizeMemeBindings,
    getTargetDisplay,
    getDisplayConfigUpdate,
    onTrayMenuNeedsRefresh: () => updateTrayMenu(),
    onConnectedPeersChange: (peers) => setConnectedOverlayPeers(peers),
    onConnectionStateChange: (state, reason = '') => setOverlayConnectionState(state, reason)
  });

  const { runtimeWindowActionsService, overlaySocketService, sendOverlaySettingsToRenderer, connectOverlaySocket, disconnectOverlaySocket } =
    runtimeWindowSocketService;

  const runtimeInteraction = createRuntimeInteractionService({
    logger,
    globalShortcut,
    autoStartRuntimeService,
    overlaySocketService,
    runtimeWindowActionsService,
    loadConfig,
    saveConfig,
    normalizeMemeBindings,
    isGuestModeEnabled,
    normalizeVolume,
    normalizeGuestMode,
    hasPairingConfig,
    getDisplayConfigUpdate,
    updateTrayMenu,
    sendOverlaySettingsToRenderer,
    setOverlayConnectionState,
    connectOverlaySocket,
    disconnectOverlaySocket
  });
  runtimeInteractionService = runtimeInteraction;

  initializeRuntimeTrayBootstrap({
    runtimeTrayService,
    appIconPath,
    createTrayInstance: (iconPath) => new Tray(nativeImage.createFromPath(iconPath)),
    buildMenuFromTemplate: (template) => Menu.buildFromTemplate(template),
    screen,
    loadConfig,
    getTargetDisplay,
    formatDisplayMenuLabel,
    supportsAutoStart: () => autoStartRuntimeService.supportsAutoStart(),
    getAutoUpdateReason: () => autoUpdateService.getReason(),
    getAutoUpdateStateLabel,
    getConnectionStateLabel,
    getConnectionReason: getOverlayConnectionReason,
    getAppVersion: () => app.getVersion(),
    hasPairingConfig,
    isGuestModeEnabled,
    buildTrayTooltip,
    buildVolumeMenuItems,
    runtimeInteractionService: runtimeInteraction,
    runtimeWindowActionsService,
    disconnectOverlaySocket,
    quitApp: () => app.quit()
  });

  const markQuittingForUpdate = (): void => {
    isQuittingForUpdate = true;
  };

  bootstrapRuntimeService({
    logger,
    app,
    ipcMain,
    globalShortcut,
    hasSingleInstanceLock,
    configPath,
    manualReloadShortcut: MANUAL_RELOAD_SHORTCUT,
    getIsQuittingForUpdate: () => isQuittingForUpdate,
    markQuittingForUpdate,
    runtimeCoreFacadeService,
    runtimeInteractionService: runtimeInteraction,
    sendOverlaySettingsToRenderer,
    connectOverlaySocket,
    createTray,
    updateTrayMenu,
    http: {
      httpRequestJson,
      isLikelyTlsError,
      formatNetworkError
    },
    autoStartRuntimeService,
    autoUpdateService,
    overlaySocketService,
    runtimeWindowActionsService
  });
}
