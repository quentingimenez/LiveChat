export const OVERLAY_PROTOCOL_VERSION = '1.0.0';

export const OVERLAY_SOCKET_EVENTS = {
  PLAY: 'overlay:play',
  STOP: 'overlay:stop',
  HEARTBEAT: 'overlay:heartbeat',
  ERROR: 'overlay:error',
} as const;

export type OverlaySocketEventName = (typeof OVERLAY_SOCKET_EVENTS)[keyof typeof OVERLAY_SOCKET_EVENTS];

export type OverlayMediaKind = 'image' | 'audio' | 'video';

export interface OverlayPlayPayload {
  jobId: string;
  media: {
    assetId: string;
    url: string;
    mime: string;
    kind: OverlayMediaKind;
    durationSec: number | null;
    isVertical: boolean;
  } | null;
  text: {
    value: string;
    enabled: boolean;
  };
  durationSec: number;
}

export interface OverlayStopPayload {
  jobId: string;
}

export interface OverlayHeartbeatPayload {
  clientId: string;
  guildId: string;
  appVersion: string;
}

export interface OverlayErrorPayload {
  jobId: string;
  code: string;
  message: string;
}

export interface OverlayPairConsumeRequest {
  code: string;
  deviceName: string;
}

export interface OverlayPairConsumeResponse {
  clientToken: string;
  clientId: string;
  guildId: string;
  apiBaseUrl: string;
}

export interface OverlayConfigResponse {
  guildId: string;
  protocolVersion: string;
  showTextDefault: boolean;
  defaultMediaTime: number;
  maxMediaTime: number | null;
}
