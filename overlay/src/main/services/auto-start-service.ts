interface LoginItemSettingsLike {
  openAtLogin?: boolean;
  openAsHidden?: boolean;
  path?: string;
  args?: string[];
  name?: string;
  enabled?: boolean;
}

interface LoginItemLaunchItemLike {
  name?: string;
  enabled?: boolean;
  path?: string;
}

interface LoginItemSettingsReadLike {
  openAtLogin?: boolean;
  executableWillLaunchAtLogin?: boolean;
  launchItems?: LoginItemLaunchItemLike[];
}

interface AppLoginItemApi {
  getLoginItemSettings(options?: { path: string; args: string[] } | undefined): LoginItemSettingsReadLike;
  setLoginItemSettings(settings: LoginItemSettingsLike): void;
}

export interface CreateAutoStartServiceOptions {
  app: AppLoginItemApi;
  platform: string;
  processExecPath: string;
  portableExecutableFileRaw?: string;
  windowsAppUserModelId: string;
  legacyWindowsAppUserModelId: string;
  onReadError?: (error: unknown) => void;
  onWriteError?: (error: unknown) => void;
  onApplied?: (enabled: boolean) => void;
}

export interface AutoStartService {
  supportsAutoStart(): boolean;
  getSystemAutoStartEnabled(): boolean;
  applyAutoStartSetting(enabled: boolean): boolean;
}

export function createAutoStartService(options: CreateAutoStartServiceOptions): AutoStartService {
  const {
    app,
    platform,
    processExecPath,
    portableExecutableFileRaw,
    windowsAppUserModelId,
    legacyWindowsAppUserModelId,
    onReadError,
    onWriteError,
    onApplied
  } = options;

  function supportsAutoStart(): boolean {
    return platform === 'win32' || platform === 'darwin';
  }

  function getWindowsAutoStartExecutablePath(): string | null {
    if (platform !== 'win32') {
      return null;
    }

    const portableExecutableFile = `${portableExecutableFileRaw || ''}`.trim();
    if (portableExecutableFile) {
      return portableExecutableFile;
    }

    return processExecPath;
  }

  function getWindowsAutoStartLoginItemOptions(): { path: string; args: string[] } | null {
    if (platform !== 'win32') {
      return null;
    }

    const executablePath = getWindowsAutoStartExecutablePath();
    if (!executablePath) {
      return null;
    }

    return {
      path: executablePath,
      args: []
    };
  }

  function normalizeWindowsExecutablePath(value: unknown): string {
    return `${value || ''}`
      .trim()
      .replace(/^"+|"+$/g, '')
      .toLowerCase();
  }

  function getSystemAutoStartEnabled(): boolean {
    if (!supportsAutoStart()) {
      return false;
    }

    try {
      if (platform === 'win32') {
        const winOptions = getWindowsAutoStartLoginItemOptions();
        const settings = app.getLoginItemSettings(winOptions || undefined);
        const expectedExecutablePath = normalizeWindowsExecutablePath(winOptions?.path);
        const launchItems = Array.isArray(settings.launchItems) ? settings.launchItems : [];

        const hasLegacyEnabledEntry =
          settings.executableWillLaunchAtLogin !== true &&
          launchItems.some((item) => {
            if (!item || item.name !== legacyWindowsAppUserModelId || item.enabled !== true) {
              return false;
            }

            if (!expectedExecutablePath) {
              return true;
            }

            return normalizeWindowsExecutablePath(item.path) === expectedExecutablePath;
          });

        return settings.executableWillLaunchAtLogin === true || settings.openAtLogin === true || hasLegacyEnabledEntry;
      }

      return app.getLoginItemSettings().openAtLogin === true;
    } catch (error) {
      if (onReadError) {
        onReadError(error);
      }
      return false;
    }
  }

  function applyAutoStartSetting(enabled: boolean): boolean {
    const nextEnabled = enabled === true;

    if (!supportsAutoStart()) {
      if (onApplied) {
        onApplied(false);
      }
      return false;
    }

    try {
      const loginItemSettings: LoginItemSettingsLike = {
        openAtLogin: nextEnabled
      };

      if (platform === 'darwin') {
        loginItemSettings.openAsHidden = true;
      }

      if (platform === 'win32') {
        const winOptions = getWindowsAutoStartLoginItemOptions();
        if (winOptions) {
          loginItemSettings.path = winOptions.path;
          loginItemSettings.args = winOptions.args;
        }

        loginItemSettings.name = windowsAppUserModelId;
        loginItemSettings.enabled = nextEnabled;
      }

      app.setLoginItemSettings(loginItemSettings);

      if (platform === 'win32' && windowsAppUserModelId !== legacyWindowsAppUserModelId) {
        const winOptions = getWindowsAutoStartLoginItemOptions();
        const legacyLoginItemSettings: LoginItemSettingsLike = {
          openAtLogin: false,
          enabled: false,
          name: legacyWindowsAppUserModelId
        };

        if (winOptions) {
          legacyLoginItemSettings.path = winOptions.path;
          legacyLoginItemSettings.args = winOptions.args;
        }

        app.setLoginItemSettings(legacyLoginItemSettings);
      }
    } catch (error) {
      if (onWriteError) {
        onWriteError(error);
      }
    }

    const systemEnabled = getSystemAutoStartEnabled();
    if (onApplied) {
      onApplied(systemEnabled);
    }

    return systemEnabled;
  }

  return {
    supportsAutoStart,
    getSystemAutoStartEnabled,
    applyAutoStartSetting
  };
}
