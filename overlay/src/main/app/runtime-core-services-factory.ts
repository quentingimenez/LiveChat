import type { App } from 'electron';
import type { Logger } from '../../shared/types';
import { createAutoStartRuntimeService, type AutoStartRuntimeService } from '../services/auto-start-runtime-service';
import { createAutoStartService } from '../services/auto-start-service';
import { createConfigService } from '../services/config-service';
import { createConnectionStateService } from '../services/connection-state-service';
import { createDisplayService, type DisplayLike } from '../services/display-service';
import { createOverlayPeersService } from '../services/overlay-peers-service';
import type { AutoUpdateService } from '../services/auto-update-service';
import { createRuntimeAutoUpdateBootstrapService } from './runtime-auto-update-bootstrap-service';
import { createRuntimeCoreFacadeService, type RuntimeCoreFacadeService } from './runtime-core-facade-service';

interface CreateRuntimeCoreServicesOptions {
  logger: Logger;
  app: App;
  platform: string;
  processExecPath: string;
  portableExecutableFileRaw?: string;
  disableAutoUpdateByEnvRawValue?: string;
  windowsAppUserModelId: string;
  legacyWindowsAppUserModelId: string;
  configPath: string;
  maxOtherActiveOverlaysInTooltip: number;
  startupAutoUpdateTimeoutMs: number;
  getAllDisplays: () => DisplayLike[];
  getPrimaryDisplay: () => DisplayLike;
  onTrayMenuNeedsRefresh: () => void;
  onStartupInstallRequested: () => void;
  onVolumeSelected: (volume: number) => void;
}

export interface RuntimeCoreServicesFactoryResult {
  runtimeCoreFacadeService: RuntimeCoreFacadeService;
  autoStartRuntimeService: AutoStartRuntimeService;
  autoUpdateService: AutoUpdateService;
}

export function createRuntimeCoreServices(options: CreateRuntimeCoreServicesOptions): RuntimeCoreServicesFactoryResult {
  const {
    logger,
    app,
    platform,
    processExecPath,
    portableExecutableFileRaw,
    disableAutoUpdateByEnvRawValue,
    windowsAppUserModelId,
    legacyWindowsAppUserModelId,
    configPath,
    maxOtherActiveOverlaysInTooltip,
    startupAutoUpdateTimeoutMs,
    getAllDisplays,
    getPrimaryDisplay,
    onTrayMenuNeedsRefresh,
    onStartupInstallRequested,
    onVolumeSelected
  } = options;

  const configService = createConfigService({
    configPath,
    onReadError: (error) => {
      logger.error('Unable to read config:', error);
    },
    onWriteError: (error) => {
      logger.error('Unable to save config:', error);
    }
  });

  const displayService = createDisplayService({
    getAllDisplays,
    getPrimaryDisplay
  });

  const overlayPeersService = createOverlayPeersService({
    maxOtherActiveOverlaysInTooltip
  });

  const connectionStateService = createConnectionStateService({
    onStateChanged: onTrayMenuNeedsRefresh
  });

  const runtimeCoreFacadeService = createRuntimeCoreFacadeService({
    configService,
    displayService,
    connectionStateService,
    overlayPeersService,
    onVolumeSelected,
    onOverlayPeersChanged: onTrayMenuNeedsRefresh
  });

  const autoStartService = createAutoStartService({
    app,
    platform,
    processExecPath,
    portableExecutableFileRaw,
    windowsAppUserModelId,
    legacyWindowsAppUserModelId,
    onReadError: (error) => {
      logger.error('Unable to read auto-start setting:', error);
    },
    onWriteError: (error) => {
      logger.error('Unable to update auto-start setting:', error);
    },
    onApplied: (enabled) => {
      runtimeCoreFacadeService.saveConfig({ autoStart: enabled });
      onTrayMenuNeedsRefresh();
    }
  });

  const autoStartRuntimeService = createAutoStartRuntimeService({
    autoStartService,
    canUseLoginItemApi: typeof app.getLoginItemSettings === 'function' && typeof app.setLoginItemSettings === 'function',
    saveAutoStartConfig: (enabled) => {
      runtimeCoreFacadeService.saveConfig({ autoStart: enabled });
    }
  });

  const autoUpdateService = createRuntimeAutoUpdateBootstrapService({
    logger,
    platform,
    isPackaged: app.isPackaged,
    portableExecutableFileRaw,
    startupTimeoutMs: startupAutoUpdateTimeoutMs,
    disableByEnvRawValue: disableAutoUpdateByEnvRawValue,
    onTrayMenuNeedsRefresh,
    onStartupInstallRequested
  });

  return {
    runtimeCoreFacadeService,
    autoStartRuntimeService,
    autoUpdateService
  };
}
