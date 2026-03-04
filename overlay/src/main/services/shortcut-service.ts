import type { BrowserWindow, GlobalShortcut } from 'electron';
import type { Dictionary } from '../../shared/types';
import type { OverlayRuntimeConfig } from './config-service';

interface MemeBindingsApplyOptions {
  strict: boolean;
  persist: boolean;
}

interface RegisterRuntimeShortcutsOptions {
  globalShortcut: GlobalShortcut;
  manualReloadShortcut: string;
  loadConfig: () => OverlayRuntimeConfig;
  getOverlayWindow: () => BrowserWindow | null;
  emitManualStopSignal: () => void;
  applyMemeBindings: (nextBindings: Dictionary<string>, options: MemeBindingsApplyOptions) => void;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
}

export function registerRuntimeShortcuts(options: RegisterRuntimeShortcutsOptions): void {
  const {
    globalShortcut,
    manualReloadShortcut,
    loadConfig,
    getOverlayWindow,
    emitManualStopSignal,
    applyMemeBindings,
    isGuestModeEnabled
  } = options;

  globalShortcut.register(manualReloadShortcut, () => {
    const config = loadConfig();
    const overlayWindow = getOverlayWindow();

    if (config.enabled && overlayWindow && !overlayWindow.isDestroyed()) {
      emitManualStopSignal();
      overlayWindow.reload();
    }
  });

  const config = loadConfig();
  applyMemeBindings(isGuestModeEnabled(config) ? {} : config.memeBindings, {
    strict: false,
    persist: false
  });
}
