export const OVERLAY_PROTOCOL_VERSION = '1.2.0';

export const OVERLAY_SOCKET_EVENTS = {
  PLAY: 'overlay:play',
  STOP: 'overlay:stop',
  HEARTBEAT: 'overlay:heartbeat',
  ERROR: 'overlay:error',
  PLAYBACK_STATE: 'overlay:playback-state',
  MEME_TRIGGER: 'overlay:meme-trigger',
  PEERS: 'overlay:peers',
} as const;

export type OverlaySocketEventName = (typeof OVERLAY_SOCKET_EVENTS)[keyof typeof OVERLAY_SOCKET_EVENTS];

export type OverlayMediaKind = 'image' | 'audio' | 'video';

export interface OverlayTweetCard {
  url: string;
  html: string;
  authorName: string;
  authorUrl: string | null;
  caption?: string | null;
}

export interface OverlayPlayPayload {
  jobId: string;
  media: {
    assetId: string;
    url: string;
    mime: string;
    kind: OverlayMediaKind;
    durationSec: number | null;
    isVertical: boolean;
    startOffsetSec?: number | null;
  } | null;
  text: {
    value: string;
    enabled: boolean;
  };
  author: {
    name: string;
    image: string | null;
    enabled: boolean;
  };
  tweetCard?: OverlayTweetCard | null;
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

export interface OverlayPlaybackStatePayload {
  jobId: string;
  state: 'playing' | 'paused' | 'ended';
  remainingMs: number | null;
}

export interface OverlayMemeTriggerPayload {
  itemId: string;
  trigger: 'shortcut' | 'ui';
}

export interface OverlayPeerInfo {
  clientId: string;
  label: string;
}

export interface OverlayPeersPayload {
  guildId: string;
  peers: OverlayPeerInfo[];
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
