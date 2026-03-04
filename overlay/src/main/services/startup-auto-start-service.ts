import fs from 'node:fs';

interface AutoStartSnapshot {
  autoStart?: unknown;
}

interface SyncAutoStartOnStartupOptions {
  configPath: string;
  persistedAutoStartEnabled: boolean;
  supportsAutoStart: boolean;
  getSystemAutoStartEnabled: () => boolean;
  applyAutoStartSetting: (enabled: boolean) => void;
  saveAutoStartConfig: (enabled: boolean) => void;
  onInspectError?: (error: unknown) => void;
}

function readPersistedAutoStartPresence(configPath: string, onInspectError?: (error: unknown) => void): boolean {
  try {
    if (!fs.existsSync(configPath)) {
      return false;
    }

    const parsed = JSON.parse(fs.readFileSync(configPath, 'utf-8')) as AutoStartSnapshot;
    return typeof parsed?.autoStart === 'boolean';
  } catch (error) {
    if (onInspectError) {
      onInspectError(error);
    }

    return false;
  }
}

export function syncAutoStartOnStartup(options: SyncAutoStartOnStartupOptions): void {
  const {
    configPath,
    persistedAutoStartEnabled,
    supportsAutoStart,
    getSystemAutoStartEnabled,
    applyAutoStartSetting,
    saveAutoStartConfig,
    onInspectError
  } = options;

  const hasPersistedAutoStart = readPersistedAutoStartPresence(configPath, onInspectError);

  if (supportsAutoStart) {
    if (hasPersistedAutoStart) {
      applyAutoStartSetting(persistedAutoStartEnabled === true);
      return;
    }

    saveAutoStartConfig(getSystemAutoStartEnabled());
    return;
  }

  if (persistedAutoStartEnabled) {
    saveAutoStartConfig(false);
  }
}
