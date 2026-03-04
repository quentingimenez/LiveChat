import type { BrowserWindow } from 'electron';
import type { OverlayRuntimeConfig } from '../services/config-service';

type FocusableWindowLike = Pick<BrowserWindow, 'isDestroyed' | 'focus'> &
  Partial<Pick<BrowserWindow, 'isMinimized' | 'restore' | 'show'>>;

interface HandleSecondInstanceOptions {
  getBoardWindow: () => FocusableWindowLike | null;
  getPairingWindow: () => FocusableWindowLike | null;
  getOverlayWindow: () => FocusableWindowLike | null;
  loadConfig: () => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  createPairingWindow: () => void;
  createBoardWindow: () => void;
}

interface HandleAppActivateOptions {
  getOverlayWindow: () => FocusableWindowLike | null;
  loadConfig: () => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  createPairingWindow: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
}

interface InitializeRuntimeOnReadyOptions {
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
}

interface RunStartupWindowFlowOptions {
  config: OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  createPairingWindow: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
}

function focusWindow(window: FocusableWindowLike | null): boolean {
  if (!window || window.isDestroyed()) {
    return false;
  }

  if (typeof window.isMinimized === 'function' && window.isMinimized()) {
    if (typeof window.restore === 'function') {
      window.restore();
    }
  }

  window.focus();
  return true;
}

function showWindow(window: FocusableWindowLike | null): boolean {
  if (!window || window.isDestroyed()) {
    return false;
  }

  if (typeof window.show === 'function') {
    window.show();
    return true;
  }

  return false;
}

export function handleSecondInstance(options: HandleSecondInstanceOptions): void {
  const {
    getBoardWindow,
    getPairingWindow,
    getOverlayWindow,
    loadConfig,
    hasPairingConfig,
    createPairingWindow,
    createBoardWindow
  } = options;

  if (focusWindow(getBoardWindow())) {
    return;
  }

  if (focusWindow(getPairingWindow())) {
    return;
  }

  if (showWindow(getOverlayWindow())) {
    return;
  }

  const config = loadConfig();
  if (!config.enabled) {
    return;
  }

  if (!hasPairingConfig(config)) {
    createPairingWindow();
    return;
  }

  createBoardWindow();
}

export function getStartupConnectionState(config: OverlayRuntimeConfig, hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean): string {
  if (!config.enabled) {
    return 'disabled';
  }

  if (!hasPairingConfig(config)) {
    return 'not_paired';
  }

  return 'connecting';
}

export function runStartupWindowFlow(options: RunStartupWindowFlowOptions): void {
  const { config, hasPairingConfig, createPairingWindow, createOverlayWindow, connectOverlaySocket } = options;

  if (!config.enabled) {
    return;
  }

  if (hasPairingConfig(config)) {
    createOverlayWindow();
    connectOverlaySocket();
    return;
  }

  createPairingWindow();
}

export function handleAppActivate(options: HandleAppActivateOptions): void {
  const { getOverlayWindow, loadConfig, hasPairingConfig, createPairingWindow, createOverlayWindow, connectOverlaySocket } = options;

  const config = loadConfig();
  if (!config.enabled) {
    return;
  }

  if (!hasPairingConfig(config)) {
    createPairingWindow();
    return;
  }

  if (!getOverlayWindow()) {
    createOverlayWindow();
    connectOverlaySocket();
  }
}

export async function initializeRuntimeOnReady(options: InitializeRuntimeOnReadyOptions): Promise<void> {
  const {
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
  } = options;

  const configBeforeAutoStart = loadConfig();
  syncAutoStartAtStartup(configBeforeAutoStart);

  createTray();
  registerRuntimeShortcuts();
  await runStartupAutoUpdateCheck();

  if (shouldAbortAfterAutoUpdate()) {
    return;
  }

  const config = loadConfig();
  setOverlayConnectionState(getStartupConnectionState(config, hasPairingConfig));
  runStartupWindowFlow({
    config,
    hasPairingConfig,
    createPairingWindow,
    createOverlayWindow,
    connectOverlaySocket
  });
}
