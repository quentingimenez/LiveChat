import type { App, BrowserWindow } from 'electron';
import type { OverlayRuntimeConfig } from '../services/config-service';
import { initializeRuntimeOnReady, handleAppActivate, handleSecondInstance } from './runtime-orchestration-service';
import { registerRuntimeAppEvents } from './runtime-app-events-service';

type FocusableWindowLike = Pick<BrowserWindow, 'isDestroyed' | 'focus'> &
  Partial<Pick<BrowserWindow, 'isMinimized' | 'restore' | 'show'>>;

interface RegisterRuntimeLifecycleOptions {
  app: App;
  hasSingleInstanceLock: boolean;
  loadConfig: () => OverlayRuntimeConfig;
  syncAutoStartAtStartup: (config: OverlayRuntimeConfig) => void;
  createTray: () => void;
  registerRuntimeShortcuts: () => void;
  runStartupAutoUpdateCheck: () => Promise<void>;
  shouldAbortAfterAutoUpdate: () => boolean;
  setOverlayConnectionState: (nextState: string, reason?: string) => void;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  createPairingWindow: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
  getOverlayWindow: () => FocusableWindowLike | null;
  getPairingWindow: () => FocusableWindowLike | null;
  getBoardWindow: () => FocusableWindowLike | null;
  createBoardWindow: () => void;
  markQuittingForUpdate: () => void;
  disconnectOverlaySocket: () => void;
  stopSocketHeartbeatLoop: () => void;
  stopOverlayKeepOnTopLoop: () => void;
  destroyBoardWindow: () => void;
  unregisterAllShortcuts: () => void;
}

export function registerRuntimeLifecycle(options: RegisterRuntimeLifecycleOptions): void {
  const {
    app,
    hasSingleInstanceLock,
    loadConfig,
    syncAutoStartAtStartup,
    createTray,
    registerRuntimeShortcuts,
    runStartupAutoUpdateCheck,
    shouldAbortAfterAutoUpdate,
    setOverlayConnectionState,
    hasPairingConfig,
    createPairingWindow,
    createOverlayWindow,
    connectOverlaySocket,
    getOverlayWindow,
    getPairingWindow,
    getBoardWindow,
    createBoardWindow,
    markQuittingForUpdate,
    disconnectOverlaySocket,
    stopSocketHeartbeatLoop,
    stopOverlayKeepOnTopLoop,
    destroyBoardWindow,
    unregisterAllShortcuts
  } = options;

  async function handleAppReady(): Promise<void> {
    await initializeRuntimeOnReady({
      loadConfig,
      syncAutoStartAtStartup,
      createTray,
      registerRuntimeShortcuts,
      runStartupAutoUpdateCheck,
      shouldAbortAfterAutoUpdate,
      setOverlayConnectionState,
      hasPairingConfig,
      createPairingWindow,
      createOverlayWindow,
      connectOverlaySocket
    });
  }

  function handleAppActivateEvent(): void {
    handleAppActivate({
      getOverlayWindow,
      loadConfig,
      hasPairingConfig,
      createPairingWindow,
      createOverlayWindow,
      connectOverlaySocket
    });
  }

  function handleSecondInstanceEvent(): void {
    handleSecondInstance({
      getBoardWindow,
      getPairingWindow,
      getOverlayWindow,
      loadConfig,
      hasPairingConfig,
      createPairingWindow,
      createBoardWindow
    });
  }

  function handleBeforeQuitForUpdateEvent(): void {
    markQuittingForUpdate();
  }

  function handleWillQuitEvent(): void {
    disconnectOverlaySocket();
    stopSocketHeartbeatLoop();
    stopOverlayKeepOnTopLoop();
    destroyBoardWindow();
    unregisterAllShortcuts();
  }

  registerRuntimeAppEvents({
    app,
    hasSingleInstanceLock,
    onSecondInstance: handleSecondInstanceEvent,
    onWhenReady: handleAppReady,
    onActivate: handleAppActivateEvent,
    onBeforeQuitForUpdate: handleBeforeQuitForUpdateEvent,
    onWillQuit: handleWillQuitEvent
  });
}
