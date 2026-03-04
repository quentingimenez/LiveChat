import { contextBridge } from 'electron';
import { IPC_CHANNELS } from '../shared/ipc';
import type { LivechatOverlayApi } from '../types/livechat-overlay';
import { invokeTyped, onTyped, sendTyped } from './bridge-api';

const api: LivechatOverlayApi = {
  getConfig: () => invokeTyped(IPC_CHANNELS.overlayGetConfig, undefined),
  rendererReady: () => sendTyped(IPC_CHANNELS.overlayRendererReady, undefined),
  onPlay: (callback) => onTyped(IPC_CHANNELS.overlayPlay, callback),
  onStop: (callback) => onTyped(IPC_CHANNELS.overlayStop, callback),
  onSettings: (callback) => onTyped(IPC_CHANNELS.overlaySettings, callback),
  reportError: (payload) => sendTyped(IPC_CHANNELS.overlayError, payload),
  reportPlaybackState: (payload) => sendTyped(IPC_CHANNELS.overlayPlaybackState, payload),
  reportPlaybackStop: (payload) => sendTyped(IPC_CHANNELS.overlayPlaybackStop, payload),
  consumePairing: (payload) => invokeTyped(IPC_CHANNELS.pairingConsume, payload),
  getMemeBindings: () => invokeTyped(IPC_CHANNELS.memeBoardGetBindings, undefined),
  setMemeBindings: (payload) => invokeTyped(IPC_CHANNELS.memeBoardSetBindings, payload),
  triggerMeme: (payload) => invokeTyped(IPC_CHANNELS.memeBoardTrigger, payload),
  stopMemePlayback: () => invokeTyped(IPC_CHANNELS.memeBoardStop, undefined)
};

contextBridge.exposeInMainWorld('livechatOverlay', api);
