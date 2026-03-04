import { createTrayLifecycleService, type TrayLifecycleService } from '../services/tray-lifecycle-service';

type TrayLifecycleOptions = Parameters<typeof createTrayLifecycleService>[0];

export interface RuntimeTrayService {
  initialize(options: TrayLifecycleOptions): void;
  createTray(): void;
  updateTrayMenu(): void;
}

export function createRuntimeTrayService(): RuntimeTrayService {
  let trayLifecycleService: TrayLifecycleService | null = null;

  function initialize(options: TrayLifecycleOptions): void {
    trayLifecycleService = createTrayLifecycleService(options);
  }

  function createTray(): void {
    if (!trayLifecycleService) {
      return;
    }

    trayLifecycleService.createTray();
  }

  function updateTrayMenu(): void {
    if (!trayLifecycleService) {
      return;
    }

    trayLifecycleService.updateTrayMenu();
  }

  return {
    initialize,
    createTray,
    updateTrayMenu
  };
}
