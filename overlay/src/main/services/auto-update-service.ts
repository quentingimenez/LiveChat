import { autoUpdater } from 'electron-updater';
import type { Logger } from '../../shared/types';

const AUTO_UPDATE_STATE_LABELS: Record<string, string> = {
  disabled: 'Désactivée',
  idle: 'Inactif',
  checking: 'Vérification...',
  downloading: 'Téléchargement...',
  downloaded: 'Téléchargée (au prochain redémarrage)',
  installing: 'Installation...',
  up_to_date: 'À jour',
  error: 'Erreur'
};

export interface CreateAutoUpdateServiceOptions {
  logger: Logger;
  platform: string;
  isPackaged: boolean;
  isPortableRuntime: () => boolean;
  startupTimeoutMs: number;
  disableByEnvRawValue?: string;
  onStateChanged: () => void;
  onStartupInstallRequested: () => void;
}

export interface AutoUpdateService {
  getState(): string;
  getReason(): string;
  getStateLabel(): string;
  runStartupAutoUpdateCheck(): Promise<void>;
}

function isFlagEnabled(value: unknown): boolean {
  const normalized = `${value || ''}`.trim().toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

function normalizeAutoUpdateReason(reason: unknown, maxLength = 90): string {
  const normalized = `${reason || ''}`.trim().replace(/\s+/g, ' ');
  if (!normalized) {
    return '';
  }

  if (normalized.length <= maxLength) {
    return normalized;
  }

  return `${normalized.slice(0, Math.max(1, maxLength - 1))}…`;
}

export function createAutoUpdateService(options: CreateAutoUpdateServiceOptions): AutoUpdateService {
  const {
    logger,
    platform,
    isPackaged,
    isPortableRuntime,
    startupTimeoutMs,
    disableByEnvRawValue,
    onStateChanged,
    onStartupInstallRequested
  } = options;

  let state = 'idle';
  let reason = '';
  let autoUpdaterInitialized = false;

  function setState(nextState: string, nextReason = ''): void {
    state = nextState;
    reason = normalizeAutoUpdateReason(nextReason);
    onStateChanged();
  }

  function getAutoUpdateDisabledReason(): string | null {
    if (platform !== 'win32') {
      return 'plateforme non supportée';
    }

    if (!isPackaged) {
      return 'mode développement';
    }

    if (isPortableRuntime()) {
      return 'désactivée (portable)';
    }

    if (isFlagEnabled(disableByEnvRawValue)) {
      return 'désactivée (env)';
    }

    return null;
  }

  function supportsStartupAutoUpdate(): boolean {
    return getAutoUpdateDisabledReason() === null;
  }

  function initializeAutoUpdater(): void {
    if (autoUpdaterInitialized) {
      return;
    }

    autoUpdaterInitialized = true;
    autoUpdater.autoDownload = true;
    autoUpdater.autoInstallOnAppQuit = true;

    autoUpdater.on('checking-for-update', () => {
      setState('checking');
    });

    autoUpdater.on('update-available', (info) => {
      const nextVersion = `${info?.version || ''}`.trim();
      const nextReason = nextVersion ? `Nouvelle version: ${nextVersion}` : '';
      setState('downloading', nextReason);
    });

    autoUpdater.on('update-not-available', () => {
      setState('up_to_date');
    });

    autoUpdater.on('download-progress', () => {
      if (state !== 'downloading') {
        setState('downloading');
      }
    });

    autoUpdater.on('update-downloaded', (info) => {
      const nextVersion = `${info?.version || ''}`.trim();
      const nextReason = nextVersion ? `Prête: ${nextVersion}` : '';
      setState('downloaded', nextReason);
    });

    autoUpdater.on('error', (error) => {
      const message = `${error?.message || error || 'unknown_update_error'}`.trim();
      logger.error('Auto-update failed:', error);
      setState('error', message || 'check_failed');
    });
  }

  async function runStartupAutoUpdateCheck(): Promise<void> {
    if (!supportsStartupAutoUpdate()) {
      const disabledReason = getAutoUpdateDisabledReason() || 'désactivée';
      setState('disabled', disabledReason);
      return;
    }

    initializeAutoUpdater();
    setState('idle');

    await new Promise<void>((resolve) => {
      let settled = false;
      let timeoutHandle: ReturnType<typeof setTimeout> | null = null;

      function cleanup(): void {
        autoUpdater.removeListener('update-not-available', onUpdateNotAvailable);
        autoUpdater.removeListener('error', onUpdateError);
        autoUpdater.removeListener('update-downloaded', onUpdateDownloaded);
      }

      function finalize(): void {
        if (settled) {
          return;
        }

        settled = true;

        if (timeoutHandle) {
          clearTimeout(timeoutHandle);
        }

        cleanup();
        resolve();
      }

      function onUpdateNotAvailable(): void {
        finalize();
      }

      function onUpdateError(): void {
        finalize();
      }

      function onUpdateDownloaded(): void {
        setState('installing');
        onStartupInstallRequested();
        autoUpdater.quitAndInstall(false, true);
        finalize();
      }

      timeoutHandle = setTimeout(() => {
        if (state === 'checking' || state === 'downloading') {
          setState(state, 'délai dépassé au démarrage');
        }
        finalize();
      }, startupTimeoutMs);

      autoUpdater.on('update-not-available', onUpdateNotAvailable);
      autoUpdater.on('error', onUpdateError);
      autoUpdater.on('update-downloaded', onUpdateDownloaded);

      autoUpdater.checkForUpdates().catch((error) => {
        logger.error('Unable to check updates on startup:', error);
        finalize();
      });
    });
  }

  return {
    getState(): string {
      return state;
    },
    getReason(): string {
      return reason;
    },
    getStateLabel(): string {
      return AUTO_UPDATE_STATE_LABELS[state] || state;
    },
    runStartupAutoUpdateCheck
  };
}
