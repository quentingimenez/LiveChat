import type { GlobalShortcut } from 'electron';
import type { Logger } from '../../shared/types';
import type { AutoStartRuntimeService } from '../services/auto-start-runtime-service';
import type { OverlayRuntimeConfig } from '../services/config-service';
import type { DisplayLike } from '../services/display-service';
import { createMemeBindingsService } from '../services/meme-bindings-service';
import { createRuntimeActionsService } from '../services/runtime-actions-service';
import type { OverlaySocketService } from '../services/socket-service';
import type { RuntimeWindowActionsService } from './runtime-window-actions-service';

interface OverlaySocketDisconnectOptions {
  nextState?: string;
  reason?: string;
  keepStatus?: boolean;
}

interface CreateRuntimeInteractionServiceOptions {
  logger: Logger;
  globalShortcut: GlobalShortcut;
  autoStartRuntimeService: AutoStartRuntimeService;
  overlaySocketService: OverlaySocketService;
  runtimeWindowActionsService: RuntimeWindowActionsService;
  loadConfig: () => OverlayRuntimeConfig;
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  normalizeVolume: (volume: unknown) => number;
  normalizeGuestMode: (value: unknown) => boolean;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  getDisplayConfigUpdate: (
    display: DisplayLike,
    index: number
  ) => {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  };
  updateTrayMenu: () => void;
  sendOverlaySettingsToRenderer: () => void;
  setOverlayConnectionState: (nextState: string, reason?: string) => void;
  connectOverlaySocket: () => void;
  disconnectOverlaySocket: (options?: OverlaySocketDisconnectOptions) => void;
}

export interface RuntimeInteractionService {
  emitManualStopSignal(): { ok: boolean; reason?: string };
  emitMemeTriggerSignal(itemId: unknown, trigger?: unknown): { ok: boolean; reason?: string };
  applyMemeBindings(
    nextBindings: unknown,
    options?: { strict?: boolean; persist?: boolean }
  ): { ok: boolean; appliedBindings: Record<string, string>; failedAccelerators: string[] };
  setEnabledAsync(enabled: boolean): Promise<void>;
  changeVolume(level: number): void;
  toggleShowText(checked: boolean): void;
  toggleAutoStart(checked: boolean): void;
  setGuestMode(checked: boolean): OverlayRuntimeConfig;
  moveOverlayToDisplay(display: DisplayLike, index: number): void;
  resetPairing(): void;
}

export function createRuntimeInteractionService(options: CreateRuntimeInteractionServiceOptions): RuntimeInteractionService {
  const {
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
  } = options;

  const memeBindingsService = createMemeBindingsService({
    logger,
    normalizeMemeBindings,
    isGuestModeEnabled: () => isGuestModeEnabled(),
    saveConfig: (nextValues) => saveConfig(nextValues),
    getSocket: () => overlaySocketService.getSocket(),
    shortcutRegistrar: globalShortcut
  });

  function emitManualStopSignal(): { ok: boolean; reason?: string } {
    return memeBindingsService.emitManualStopSignal();
  }

  function emitMemeTriggerSignal(itemId: unknown, trigger: unknown = 'shortcut'): { ok: boolean; reason?: string } {
    return memeBindingsService.emitMemeTriggerSignal(itemId, trigger);
  }

  function unregisterMemeShortcuts(): void {
    memeBindingsService.unregisterMemeShortcuts();
  }

  function applyMemeBindings(
    nextBindings: unknown,
    options: { strict?: boolean; persist?: boolean } = {}
  ): { ok: boolean; appliedBindings: Record<string, string>; failedAccelerators: string[] } {
    return memeBindingsService.applyMemeBindings(nextBindings, options);
  }

  const runtimeActionsService = createRuntimeActionsService({
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
    destroyOverlayWindow: () => {
      runtimeWindowActionsService.destroyOverlayWindow();
    },
    destroyBoardWindow: () => {
      runtimeWindowActionsService.destroyBoardWindow();
    },
    setOverlayConnectionState,
    createPairingWindow: () => {
      runtimeWindowActionsService.createPairingWindow();
    },
    createOverlayWindow: () => {
      runtimeWindowActionsService.createOverlayWindow();
    },
    connectOverlaySocket,
    clearPendingPlayback: () => {
      overlaySocketService.clearPendingPlayback();
    },
    unregisterMemeShortcuts,
    applyMemeBindings,
    moveOverlayWindowToDisplay: (display) => {
      runtimeWindowActionsService.moveOverlayWindowToDisplay(display);
    }
  });

  function toggleAutoStart(checked: boolean): void {
    autoStartRuntimeService.applyAutoStartSetting(checked);
  }

  return {
    emitManualStopSignal,
    emitMemeTriggerSignal,
    applyMemeBindings,
    setEnabledAsync: (enabled) => runtimeActionsService.setEnabledAsync(enabled),
    changeVolume: (level) => {
      runtimeActionsService.changeVolume(level);
    },
    toggleShowText: (checked) => {
      runtimeActionsService.toggleShowText(checked);
    },
    toggleAutoStart,
    setGuestMode: (checked) => runtimeActionsService.setGuestMode(checked),
    moveOverlayToDisplay: (display, index) => {
      runtimeActionsService.moveOverlayToDisplay(display, index);
    },
    resetPairing: () => {
      runtimeActionsService.resetPairing();
    }
  };
}
