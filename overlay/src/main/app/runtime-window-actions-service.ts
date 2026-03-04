import type { OverlayRuntimeConfig } from '../services/config-service';
import type { DisplayLike } from '../services/display-service';
import type { AuxiliaryWindowService } from '../windows/auxiliary-window-service';
import type { OverlayWindowService } from '../windows/overlay-window-service';

interface CreateRuntimeWindowActionsServiceOptions {
  loadConfig: () => OverlayRuntimeConfig;
  hasPairingConfig: (config?: OverlayRuntimeConfig) => boolean;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  auxiliaryWindowService: AuxiliaryWindowService;
  overlayWindowService: OverlayWindowService;
}

export interface RuntimeWindowActionsService {
  getOverlayWindow: OverlayWindowService['getOverlayWindow'];
  getPairingWindow: AuxiliaryWindowService['getPairingWindow'];
  getBoardWindow: AuxiliaryWindowService['getBoardWindow'];
  stopKeepOnTopLoop: OverlayWindowService['stopKeepOnTopLoop'];
  createOverlayWindow: OverlayWindowService['createOverlayWindow'];
  destroyOverlayWindow: OverlayWindowService['destroyOverlayWindow'];
  createPairingWindow: AuxiliaryWindowService['createPairingWindow'];
  closePairingWindow: AuxiliaryWindowService['closePairingWindow'];
  createBoardWindow(): void;
  destroyBoardWindow: AuxiliaryWindowService['destroyBoardWindow'];
  moveOverlayWindowToDisplay: OverlayWindowService['moveOverlayWindowToDisplay'];
}

export function createRuntimeWindowActionsService(
  options: CreateRuntimeWindowActionsServiceOptions
): RuntimeWindowActionsService {
  const { loadConfig, hasPairingConfig, isGuestModeEnabled, auxiliaryWindowService, overlayWindowService } = options;

  function createBoardWindow(): void {
    const config = loadConfig();

    if (isGuestModeEnabled(config)) {
      return;
    }

    if (!hasPairingConfig(config)) {
      auxiliaryWindowService.createPairingWindow();
      return;
    }

    auxiliaryWindowService.createBoardWindow();
  }

  return {
    getOverlayWindow: () => overlayWindowService.getOverlayWindow(),
    getPairingWindow: () => auxiliaryWindowService.getPairingWindow(),
    getBoardWindow: () => auxiliaryWindowService.getBoardWindow(),
    stopKeepOnTopLoop: () => overlayWindowService.stopKeepOnTopLoop(),
    createOverlayWindow: () => overlayWindowService.createOverlayWindow(),
    destroyOverlayWindow: () => overlayWindowService.destroyOverlayWindow(),
    createPairingWindow: () => auxiliaryWindowService.createPairingWindow(),
    closePairingWindow: () => auxiliaryWindowService.closePairingWindow(),
    createBoardWindow,
    destroyBoardWindow: () => auxiliaryWindowService.destroyBoardWindow(),
    moveOverlayWindowToDisplay: (display: DisplayLike) => {
      overlayWindowService.moveOverlayWindowToDisplay(display);
    }
  };
}
