import type { App, GlobalShortcut, IpcMain } from 'electron';
import type { Logger } from '../../shared/types';
import type { OverlayRuntimeConfig } from '../services/config-service';
import type { AutoStartRuntimeService } from '../services/auto-start-runtime-service';
import type { AutoUpdateService } from '../services/auto-update-service';
import type { OverlaySocketService } from '../services/socket-service';
import { registerIpcHandlers } from '../ipc/register-ipc-handlers';
import type { RuntimeCoreFacadeService } from './runtime-core-facade-service';
import type { RuntimeInteractionService } from './runtime-interaction-service';
import { registerRuntimeLifecycle } from './runtime-lifecycle-service';
import { createRuntimeStartupHooksService } from './runtime-startup-hooks-service';
import type { RuntimeWindowActionsService } from './runtime-window-actions-service';

type RuntimeCoreBootstrapFacade = Pick<
  RuntimeCoreFacadeService,
  'loadConfig' | 'saveConfig' | 'hasPairingConfig' | 'isGuestModeEnabled' | 'normalizeMemeBindings' | 'normalizeServerUrl' | 'setOverlayConnectionState'
>;

interface RuntimeHttpHandlers {
  httpRequestJson: (
    url: string,
    payload: { code: string; deviceName?: string },
    options?: { rejectUnauthorized?: boolean; timeoutMs?: number }
  ) => Promise<{ statusCode: number; body: string }>;
  isLikelyTlsError: (error: unknown) => boolean;
  formatNetworkError: (error: unknown, endpoint: string) => string;
}

interface BootstrapRuntimeServiceOptions {
  logger: Logger;
  app: App;
  ipcMain: IpcMain;
  globalShortcut: GlobalShortcut;
  hasSingleInstanceLock: boolean;
  configPath: string;
  manualReloadShortcut: string;
  getIsQuittingForUpdate: () => boolean;
  markQuittingForUpdate: () => void;
  runtimeCoreFacadeService: RuntimeCoreBootstrapFacade;
  runtimeInteractionService: RuntimeInteractionService;
  sendOverlaySettingsToRenderer: () => void;
  connectOverlaySocket: () => void;
  createTray: () => void;
  updateTrayMenu: () => void;
  http: RuntimeHttpHandlers;
  autoStartRuntimeService: AutoStartRuntimeService;
  autoUpdateService: Pick<AutoUpdateService, 'runStartupAutoUpdateCheck'>;
  overlaySocketService: OverlaySocketService;
  runtimeWindowActionsService: RuntimeWindowActionsService;
}

export function bootstrapRuntimeService(options: BootstrapRuntimeServiceOptions): void {
  const {
    logger,
    app,
    ipcMain,
    globalShortcut,
    hasSingleInstanceLock,
    configPath,
    manualReloadShortcut,
    getIsQuittingForUpdate,
    markQuittingForUpdate,
    runtimeCoreFacadeService,
    runtimeInteractionService,
    sendOverlaySettingsToRenderer,
    connectOverlaySocket,
    createTray,
    updateTrayMenu,
    http,
    autoStartRuntimeService,
    autoUpdateService,
    overlaySocketService,
    runtimeWindowActionsService
  } = options;

  registerIpcHandlers({
    logger,
    ipcMain,
    loadConfig: runtimeCoreFacadeService.loadConfig,
    saveConfig: runtimeCoreFacadeService.saveConfig,
    hasPairingConfig: runtimeCoreFacadeService.hasPairingConfig,
    isGuestModeEnabled: runtimeCoreFacadeService.isGuestModeEnabled,
    normalizeMemeBindings: runtimeCoreFacadeService.normalizeMemeBindings,
    normalizeServerUrl: runtimeCoreFacadeService.normalizeServerUrl,
    applyMemeBindings: runtimeInteractionService.applyMemeBindings,
    emitMemeTriggerSignal: runtimeInteractionService.emitMemeTriggerSignal,
    emitManualStopSignal: runtimeInteractionService.emitManualStopSignal,
    sendOverlaySettingsToRenderer,
    getOverlaySocket: () => overlaySocketService.getSocket(),
    setPendingPlaybackStatePayload: (payload) => {
      overlaySocketService.setPendingPlaybackStatePayload(payload);
    },
    setPendingPlaybackStopPayload: (payload) => {
      overlaySocketService.setPendingPlaybackStopPayload(payload);
    },
    httpRequestJson: http.httpRequestJson,
    isLikelyTlsError: http.isLikelyTlsError,
    formatNetworkError: http.formatNetworkError,
    setGuestMode: runtimeInteractionService.setGuestMode,
    createOverlayWindow: () => {
      runtimeWindowActionsService.createOverlayWindow();
    },
    connectOverlaySocket,
    closePairingWindow: () => {
      runtimeWindowActionsService.closePairingWindow();
    },
    updateTrayMenu
  });

  const runtimeStartupHooksService = createRuntimeStartupHooksService({
    logger,
    configPath,
    autoStartRuntimeService,
    saveConfig: runtimeCoreFacadeService.saveConfig,
    globalShortcut,
    manualReloadShortcut,
    loadConfig: runtimeCoreFacadeService.loadConfig,
    getOverlayWindow: () => runtimeWindowActionsService.getOverlayWindow(),
    emitManualStopSignal: runtimeInteractionService.emitManualStopSignal,
    applyMemeBindings: runtimeInteractionService.applyMemeBindings,
    isGuestModeEnabled: runtimeCoreFacadeService.isGuestModeEnabled
  });

  registerRuntimeLifecycle({
    app,
    hasSingleInstanceLock,
    loadConfig: runtimeCoreFacadeService.loadConfig,
    syncAutoStartAtStartup: (config: OverlayRuntimeConfig) => runtimeStartupHooksService.syncAutoStartWithSystemAtStartup(config),
    createTray,
    registerRuntimeShortcuts: () => runtimeStartupHooksService.registerMainRuntimeShortcuts(),
    runStartupAutoUpdateCheck: () => autoUpdateService.runStartupAutoUpdateCheck(),
    shouldAbortAfterAutoUpdate: () => getIsQuittingForUpdate(),
    setOverlayConnectionState: runtimeCoreFacadeService.setOverlayConnectionState,
    hasPairingConfig: runtimeCoreFacadeService.hasPairingConfig,
    createPairingWindow: () => {
      runtimeWindowActionsService.createPairingWindow();
    },
    createOverlayWindow: () => {
      runtimeWindowActionsService.createOverlayWindow();
    },
    connectOverlaySocket,
    getOverlayWindow: () => runtimeWindowActionsService.getOverlayWindow(),
    getPairingWindow: () => runtimeWindowActionsService.getPairingWindow(),
    getBoardWindow: () => runtimeWindowActionsService.getBoardWindow(),
    createBoardWindow: () => {
      runtimeWindowActionsService.createBoardWindow();
    },
    markQuittingForUpdate,
    disconnectOverlaySocket: () => {
      overlaySocketService.disconnect();
    },
    stopSocketHeartbeatLoop: () => {
      overlaySocketService.stopHeartbeatLoop();
    },
    stopOverlayKeepOnTopLoop: () => {
      runtimeWindowActionsService.stopKeepOnTopLoop();
    },
    destroyBoardWindow: () => {
      runtimeWindowActionsService.destroyBoardWindow();
    },
    unregisterAllShortcuts: () => {
      globalShortcut.unregisterAll();
    }
  });
}
