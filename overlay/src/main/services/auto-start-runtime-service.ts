import type { AutoStartService } from './auto-start-service';

interface CreateAutoStartRuntimeServiceOptions {
  autoStartService: AutoStartService;
  canUseLoginItemApi: boolean;
  saveAutoStartConfig: (enabled: boolean) => void;
}

export interface AutoStartRuntimeService {
  supportsAutoStart(): boolean;
  getSystemAutoStartEnabled(): boolean;
  applyAutoStartSetting(enabled: boolean): boolean;
}

export function createAutoStartRuntimeService(options: CreateAutoStartRuntimeServiceOptions): AutoStartRuntimeService {
  const { autoStartService, canUseLoginItemApi, saveAutoStartConfig } = options;

  function supportsAutoStart(): boolean {
    if (!canUseLoginItemApi) {
      return false;
    }

    return autoStartService.supportsAutoStart();
  }

  function getSystemAutoStartEnabled(): boolean {
    if (!supportsAutoStart()) {
      return false;
    }

    return autoStartService.getSystemAutoStartEnabled();
  }

  function applyAutoStartSetting(enabled: boolean): boolean {
    if (!supportsAutoStart()) {
      saveAutoStartConfig(false);
      return false;
    }

    return autoStartService.applyAutoStartSetting(enabled);
  }

  return {
    supportsAutoStart,
    getSystemAutoStartEnabled,
    applyAutoStartSetting
  };
}
