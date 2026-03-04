import { IPC_CHANNELS } from '../../shared/ipc';
import { createPairingConsumeService } from '../services/pairing-consume-service';
import type { PairingIpcHandlerOptions } from './ipc-handler-types';

export function registerPairingIpcHandler(options: PairingIpcHandlerOptions): void {
  const {
    ipcMain,
    saveConfig,
    hasPairingConfig,
    normalizeServerUrl,
    httpRequestJson,
    isLikelyTlsError,
    formatNetworkError,
    setGuestMode,
    sendOverlaySettingsToRenderer,
    createOverlayWindow,
    connectOverlaySocket,
    closePairingWindow,
    updateTrayMenu
  } = options;
  const pairingConsumeService = createPairingConsumeService({
    saveConfig,
    hasPairingConfig,
    normalizeServerUrl,
    httpRequestJson,
    isLikelyTlsError,
    formatNetworkError,
    setGuestMode,
    sendOverlaySettingsToRenderer,
    createOverlayWindow,
    connectOverlaySocket,
    closePairingWindow,
    updateTrayMenu
  });

  ipcMain.handle(IPC_CHANNELS.pairingConsume, async (_event, payload) => {
    return pairingConsumeService.consumePairing(payload);
  });
}
