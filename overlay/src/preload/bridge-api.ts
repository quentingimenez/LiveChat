import { ipcRenderer } from 'electron';
import type { IpcRendererEvent } from 'electron';
import { IPC_CHANNELS } from '../shared/ipc';
import type {
  IpcEventChannel,
  IpcEventPayloadMap,
  IpcInvokeChannel,
  IpcInvokeRequestMap,
  IpcInvokeResponseMap,
  IpcSendChannel,
  IpcSendPayloadMap
} from '../shared/ipc';

export function invokeTyped<TChannel extends IpcInvokeChannel>(
  channel: TChannel,
  payload: IpcInvokeRequestMap[TChannel]
): Promise<IpcInvokeResponseMap[TChannel]> {
  return ipcRenderer.invoke(channel, payload) as Promise<IpcInvokeResponseMap[TChannel]>;
}

export function sendTyped<TChannel extends IpcSendChannel>(
  channel: TChannel,
  payload: IpcSendPayloadMap[TChannel]
): void {
  ipcRenderer.send(channel, payload);
}

export function onTyped<TChannel extends IpcEventChannel>(
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

export const preloadChannels = IPC_CHANNELS;
