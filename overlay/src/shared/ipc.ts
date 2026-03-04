import type {
  MemeBoardActionResponse,
  MemeBoardBindingsResponse,
  MemeBoardSetBindingsRequest,
  MemeBoardSetBindingsResponse,
  MemeBoardTriggerRequest
} from './meme-board';
import type {
  OverlayConfig,
  OverlayErrorPayload,
  OverlayPlayPayload,
  OverlayPlaybackStatePayload,
  OverlayPlaybackStopPayload,
  OverlaySettingsPayload,
  OverlayStopPayload
} from './overlay-config';
import type { PairingConsumeRequest, PairingConsumeResponse } from './pairing';

export const IPC_CHANNELS = {
  overlayGetConfig: 'overlay:get-config',
  overlayRendererReady: 'overlay:renderer-ready',
  overlayPlay: 'overlay:play',
  overlayStop: 'overlay:stop',
  overlaySettings: 'overlay:settings',
  overlayError: 'overlay:error',
  overlayPlaybackState: 'overlay:playback-state',
  overlayPlaybackStop: 'overlay:playback-stop',
  pairingConsume: 'pairing:consume',
  memeBoardGetBindings: 'meme-board:get-bindings',
  memeBoardSetBindings: 'meme-board:set-bindings',
  memeBoardTrigger: 'meme-board:trigger',
  memeBoardStop: 'meme-board:stop'
} as const;

export type IpcInvokeRequestMap = {
  [IPC_CHANNELS.overlayGetConfig]: undefined;
  [IPC_CHANNELS.pairingConsume]: PairingConsumeRequest;
  [IPC_CHANNELS.memeBoardGetBindings]: undefined;
  [IPC_CHANNELS.memeBoardSetBindings]: MemeBoardSetBindingsRequest;
  [IPC_CHANNELS.memeBoardTrigger]: MemeBoardTriggerRequest;
  [IPC_CHANNELS.memeBoardStop]: undefined;
};

export type IpcInvokeResponseMap = {
  [IPC_CHANNELS.overlayGetConfig]: OverlayConfig;
  [IPC_CHANNELS.pairingConsume]: PairingConsumeResponse;
  [IPC_CHANNELS.memeBoardGetBindings]: MemeBoardBindingsResponse;
  [IPC_CHANNELS.memeBoardSetBindings]: MemeBoardSetBindingsResponse;
  [IPC_CHANNELS.memeBoardTrigger]: MemeBoardActionResponse;
  [IPC_CHANNELS.memeBoardStop]: MemeBoardActionResponse;
};

export type IpcSendPayloadMap = {
  [IPC_CHANNELS.overlayRendererReady]: undefined;
  [IPC_CHANNELS.overlayError]: OverlayErrorPayload;
  [IPC_CHANNELS.overlayPlaybackState]: OverlayPlaybackStatePayload;
  [IPC_CHANNELS.overlayPlaybackStop]: OverlayPlaybackStopPayload;
};

export type IpcEventPayloadMap = {
  [IPC_CHANNELS.overlayPlay]: OverlayPlayPayload;
  [IPC_CHANNELS.overlayStop]: OverlayStopPayload;
  [IPC_CHANNELS.overlaySettings]: OverlaySettingsPayload;
};

export type IpcInvokeChannel = keyof IpcInvokeRequestMap;
export type IpcSendChannel = keyof IpcSendPayloadMap;
export type IpcEventChannel = keyof IpcEventPayloadMap;
