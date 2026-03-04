import { createAutoUpdateService, type AutoUpdateService } from '../services/auto-update-service';
import type { Logger } from '../../shared/types';

interface CreateRuntimeAutoUpdateBootstrapServiceOptions {
  logger: Logger;
  platform: string;
  isPackaged: boolean;
  portableExecutableFileRaw?: string;
  startupTimeoutMs: number;
  disableByEnvRawValue?: string;
  onTrayMenuNeedsRefresh: () => void;
  onStartupInstallRequested: () => void;
}

export function createRuntimeAutoUpdateBootstrapService(
  options: CreateRuntimeAutoUpdateBootstrapServiceOptions
): AutoUpdateService {
  const {
    logger,
    platform,
    isPackaged,
    portableExecutableFileRaw,
    startupTimeoutMs,
    disableByEnvRawValue,
    onTrayMenuNeedsRefresh,
    onStartupInstallRequested
  } = options;

  function isPortableRuntime(): boolean {
    if (platform !== 'win32') {
      return false;
    }

    return `${portableExecutableFileRaw || ''}`.trim() !== '';
  }

  return createAutoUpdateService({
    logger,
    platform,
    isPackaged,
    isPortableRuntime,
    startupTimeoutMs,
    disableByEnvRawValue,
    onStateChanged: () => {
      onTrayMenuNeedsRefresh();
    },
    onStartupInstallRequested
  });
}
