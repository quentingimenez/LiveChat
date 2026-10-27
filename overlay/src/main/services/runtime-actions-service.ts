import type { Dictionary } from '../../shared/types';
import type { OverlayRuntimeConfig } from './config-service';
import type { DisplayLike } from './display-service';

interface OverlaySocketDisconnectOptions {
  nextState?: string;
  reason?: string;
  keepStatus?: boolean;
}

interface MemeBindingsApplyOptions {
  strict?: boolean;
  persist?: boolean;
}

interface RuntimeActionsServiceOptions {
  loadConfig: () => OverlayRuntimeConfig;
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  normalizeVolume: (volume: unknown) => number;
  normalizeGuestMode: (value: unknown) => boolean;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  getDisplayConfigUpdate: (display: DisplayLike, index: number) => {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  };
  updateTrayMenu: () => void;
  sendOverlaySettingsToRenderer: () => void;
  disconnectOverlaySocket: (options?: OverlaySocketDisconnectOptions) => void;
  destroyOverlayWindow: () => void;
  destroyBoardWindow: () => void;
  setOverlayConnectionState: (nextState: string, reason?: string) => void;
  createPairingWindow: () => void;
  createOverlayWindow: () => void;
  connectOverlaySocket: () => void;
  clearPendingPlayback: () => void;
  unregisterMemeShortcuts: () => void;
  applyMemeBindings: (nextBindings: Dictionary<string>, options?: MemeBindingsApplyOptions) => unknown;
  moveOverlayWindowToDisplay: (display: DisplayLike) => void;
}

export interface RuntimeActionsService {
  setEnabledAsync(enabled: boolean): Promise<void>;
  changeVolume(level: number): void;
  toggleShowText(checked: boolean): void;
  setGuestMode(checked: boolean): OverlayRuntimeConfig;
  moveOverlayToDisplay(display: DisplayLike, index: number): void;
  resetPairing(): void;
}

export function createRuntimeActionsService(options: RuntimeActionsServiceOptions): RuntimeActionsService {
  const {
    loadConfig,
    saveConfig,
    normalizeVolume,
    normalizeGuestMode,
    hasPairingConfig,
    isGuestModeEnabled,
    getDisplayConfigUpdate,
    updateTrayMenu,
    sendOverlaySettingsToRenderer,
    disconnectOverlaySocket,
    destroyOverlayWindow,
    destroyBoardWindow,
    setOverlayConnectionState,
    createPairingWindow,
    createOverlayWindow,
    connectOverlaySocket,
    clearPendingPlayback,
    unregisterMemeShortcuts,
    applyMemeBindings,
    moveOverlayWindowToDisplay
  } = options;

  async function setEnabledAsync(enabled: boolean): Promise<void> {
    saveConfig({ enabled });

    if (!enabled) {
      disconnectOverlaySocket({ nextState: 'disabled' });
      destroyOverlayWindow();
      destroyBoardWindow();
      return;
    }

    if (!hasPairingConfig()) {
      setOverlayConnectionState('not_paired');
      createPairingWindow();
      return;
    }

    createOverlayWindow();
    connectOverlaySocket();
  }

  function changeVolume(level: number): void {
    saveConfig({ volume: normalizeVolume(level) });
    sendOverlaySettingsToRenderer();
    updateTrayMenu();
  }

  function toggleShowText(checked: boolean): void {
    saveConfig({ showText: checked });
    sendOverlaySettingsToRenderer();
    updateTrayMenu();
  }

  function setGuestMode(checked: boolean): OverlayRuntimeConfig {
    const config = saveConfig({
      guestMode: normalizeGuestMode(checked)
    });

    if (isGuestModeEnabled(config)) {
      destroyBoardWindow();
      clearPendingPlayback();
      unregisterMemeShortcuts();
    } else {
      applyMemeBindings(config.memeBindings, {
        strict: false,
        persist: false
      });
    }

    updateTrayMenu();
    return config;
  }

  function moveOverlayToDisplay(display: DisplayLike, index: number): void {
    saveConfig(getDisplayConfigUpdate(display, index));
    moveOverlayWindowToDisplay(display);
    updateTrayMenu();
  }

  function resetPairing(): void {
    saveConfig({
      serverUrl: null,
      clientToken: null,
      guildId: null,
      clientId: null,
      authorName: null,
      deviceName: null,
      guestMode: false
    });

    disconnectOverlaySocket({ nextState: 'not_paired' });
    destroyOverlayWindow();
    destroyBoardWindow();

    if (loadConfig().enabled) {
      createPairingWindow();
    }
  }

  return {
    setEnabledAsync,
    changeVolume,
    toggleShowText,
    setGuestMode,
    moveOverlayToDisplay,
    resetPairing
  };
}
