import { contextBridge, ipcRenderer } from 'electron';
import type { IpcRendererEvent } from 'electron';
import type {
  IpcEventChannel,
  IpcEventPayloadMap,
  IpcInvokeChannel,
  IpcInvokeRequestMap,
  IpcInvokeResponseMap,
  IpcSendChannel,
  IpcSendPayloadMap
} from '../shared/ipc';
import type { LivechatOverlayApi } from '../types/livechat-overlay';

function invokeTyped<TChannel extends IpcInvokeChannel>(
  channel: TChannel,
  payload: IpcInvokeRequestMap[TChannel]
): Promise<IpcInvokeResponseMap[TChannel]> {
  return ipcRenderer.invoke(channel, payload) as Promise<IpcInvokeResponseMap[TChannel]>;
}

function sendTyped<TChannel extends IpcSendChannel>(channel: TChannel, payload: IpcSendPayloadMap[TChannel]): void {
  ipcRenderer.send(channel, payload);
}

function onTyped<TChannel extends IpcEventChannel>(
  channel: TChannel,
  callback: (payload: IpcEventPayloadMap[TChannel]) => void
): () => void {
  const listener = (_event: IpcRendererEvent, payload: IpcEventPayloadMap[TChannel]) => {
    callback(payload);
  };

  ipcRenderer.on(channel, listener);

  return () => {
    ipcRenderer.removeListener(channel, listener);
  };
}

const api: LivechatOverlayApi = {
  getConfig: () => invokeTyped('overlay:get-config', undefined),
  rendererReady: () => sendTyped('overlay:renderer-ready', undefined),
  onPlay: (callback) => onTyped('overlay:play', callback),
  onStop: (callback) => onTyped('overlay:stop', callback),
  onSettings: (callback) => onTyped('overlay:settings', callback),
  reportError: (payload) => sendTyped('overlay:error', payload),
  reportPlaybackState: (payload) => sendTyped('overlay:playback-state', payload),
  reportPlaybackStop: (payload) => sendTyped('overlay:playback-stop', payload),
  consumePairing: (payload) => invokeTyped('pairing:consume', payload),
  getMemeBindings: () => invokeTyped('meme-board:get-bindings', undefined),
  setMemeBindings: (payload) => invokeTyped('meme-board:set-bindings', payload),
  triggerMeme: (payload) => invokeTyped('meme-board:trigger', payload),
  stopMemePlayback: () => invokeTyped('meme-board:stop', undefined)
};

contextBridge.exposeInMainWorld('livechatOverlay', api);
