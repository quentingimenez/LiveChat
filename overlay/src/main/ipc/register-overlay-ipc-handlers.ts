import { IPC_CHANNELS } from '../../shared/ipc';
import { createMemeBoardIpcService } from '../services/meme-board-ipc-service';
import { createOverlayConfigIpcService } from '../services/overlay-config-ipc-service';
import { createOverlayPlaybackForwardingService } from '../services/overlay-playback-forwarding-service';
import type { OverlayIpcHandlerOptions } from './ipc-handler-types';

export function registerOverlayIpcHandlers(options: OverlayIpcHandlerOptions): void {
  const {
    logger,
    ipcMain,
    loadConfig,
    isGuestModeEnabled,
    normalizeMemeBindings,
    applyMemeBindings,
    emitMemeTriggerSignal,
    emitManualStopSignal,
    sendOverlaySettingsToRenderer,
    getOverlaySocket,
    setPendingPlaybackStatePayload,
    setPendingPlaybackStopPayload
  } = options;
  const overlayPlaybackForwardingService = createOverlayPlaybackForwardingService({
    logger,
    isGuestModeEnabled: () => isGuestModeEnabled(),
    getOverlaySocket,
    setPendingPlaybackStatePayload,
    setPendingPlaybackStopPayload
  });
  const overlayConfigIpcService = createOverlayConfigIpcService({
    loadConfig,
    isGuestModeEnabled,
    normalizeMemeBindings
  });
  const memeBoardIpcService = createMemeBoardIpcService({
    loadConfig,
    normalizeMemeBindings,
    applyMemeBindings,
    emitMemeTriggerSignal,
    emitManualStopSignal
  });

  ipcMain.handle(IPC_CHANNELS.overlayGetConfig, () => {
    return overlayConfigIpcService.getConfig();
  });

  ipcMain.handle(IPC_CHANNELS.memeBoardGetBindings, () => {
    return memeBoardIpcService.getBindings();
  });

  ipcMain.handle(IPC_CHANNELS.memeBoardSetBindings, (_event, payload) => {
    return memeBoardIpcService.setBindings(payload);
  });

  ipcMain.handle(IPC_CHANNELS.memeBoardTrigger, (_event, payload) => {
    return memeBoardIpcService.trigger(payload);
  });

  ipcMain.handle(IPC_CHANNELS.memeBoardStop, () => memeBoardIpcService.stop());

  ipcMain.on(IPC_CHANNELS.overlayRendererReady, () => {
    sendOverlaySettingsToRenderer();
  });

  ipcMain.on(IPC_CHANNELS.overlayError, (_event, payload) => {
    overlayPlaybackForwardingService.handleOverlayError(payload);
  });

  ipcMain.on(IPC_CHANNELS.overlayPlaybackState, (_event, payload) => {
    overlayPlaybackForwardingService.handleOverlayPlaybackState(payload);
  });

  ipcMain.on(IPC_CHANNELS.overlayPlaybackStop, (_event, payload) => {
    overlayPlaybackForwardingService.handleOverlayPlaybackStop(payload);
  });
}
