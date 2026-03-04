import type { BrowserWindow, GlobalShortcut } from 'electron';
import type { Logger } from '../../shared/types';
import type { AutoStartRuntimeService } from '../services/auto-start-runtime-service';
import type { OverlayRuntimeConfig } from '../services/config-service';
import { registerRuntimeShortcuts } from '../services/shortcut-service';
import { syncAutoStartOnStartup } from '../services/startup-auto-start-service';

interface CreateRuntimeStartupHooksServiceOptions {
  logger: Logger;
  configPath: string;
  autoStartRuntimeService: AutoStartRuntimeService;
  saveConfig: (nextValues: Partial<OverlayRuntimeConfig>) => OverlayRuntimeConfig;
  globalShortcut: GlobalShortcut;
  manualReloadShortcut: string;
  loadConfig: () => OverlayRuntimeConfig;
  getOverlayWindow: () => BrowserWindow | null;
  emitManualStopSignal: () => { ok: boolean; reason?: string };
  applyMemeBindings: (
    nextBindings: Record<string, string>,
    options?: { strict?: boolean; persist?: boolean }
  ) => { ok: boolean; appliedBindings: Record<string, string>; failedAccelerators: string[] };
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
}

export interface RuntimeStartupHooksService {
  syncAutoStartWithSystemAtStartup(config: OverlayRuntimeConfig): void;
  registerMainRuntimeShortcuts(): void;
}

export function createRuntimeStartupHooksService(
  options: CreateRuntimeStartupHooksServiceOptions
): RuntimeStartupHooksService {
  const {
    logger,
    configPath,
    autoStartRuntimeService,
    saveConfig,
    globalShortcut,
    manualReloadShortcut,
    loadConfig,
    getOverlayWindow,
    emitManualStopSignal,
    applyMemeBindings,
    isGuestModeEnabled
  } = options;

  function syncAutoStartWithSystemAtStartup(config: OverlayRuntimeConfig): void {
    syncAutoStartOnStartup({
      configPath,
      persistedAutoStartEnabled: config.autoStart === true,
      supportsAutoStart: autoStartRuntimeService.supportsAutoStart(),
      getSystemAutoStartEnabled: () => autoStartRuntimeService.getSystemAutoStartEnabled(),
      applyAutoStartSetting: (enabled) => autoStartRuntimeService.applyAutoStartSetting(enabled),
      saveAutoStartConfig: (enabled) => {
        saveConfig({ autoStart: enabled });
      },
      onInspectError: (error) => {
        logger.error('Unable to inspect persisted auto-start preference:', error);
      }
    });
  }

  function registerMainRuntimeShortcuts(): void {
    registerRuntimeShortcuts({
      globalShortcut,
      manualReloadShortcut,
      loadConfig,
      getOverlayWindow,
      emitManualStopSignal: () => {
        emitManualStopSignal();
      },
      applyMemeBindings: (nextBindings, shortcutOptions) => {
        applyMemeBindings(nextBindings, shortcutOptions);
      },
      isGuestModeEnabled
    });
  }

  return {
    syncAutoStartWithSystemAtStartup,
    registerMainRuntimeShortcuts
  };
}
