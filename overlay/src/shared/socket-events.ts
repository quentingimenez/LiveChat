export const OVERLAY_SOCKET_EVENTS = {
  PLAY: 'overlay:play',
  STOP: 'overlay:stop',
  HEARTBEAT: 'overlay:heartbeat',
  ERROR: 'overlay:error',
  PLAYBACK_STATE: 'overlay:playback-state',
  MEME_TRIGGER: 'overlay:meme-trigger',
  PEERS: 'overlay:peers'
} as const;

export type OverlaySocketEvent = (typeof OVERLAY_SOCKET_EVENTS)[keyof typeof OVERLAY_SOCKET_EVENTS];
