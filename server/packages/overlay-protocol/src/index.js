"use strict";

Object.defineProperty(exports, "__esModule", { value: true });
exports.OVERLAY_SOCKET_EVENTS = exports.OVERLAY_PROTOCOL_VERSION = void 0;

exports.OVERLAY_PROTOCOL_VERSION = "1.2.0";

exports.OVERLAY_SOCKET_EVENTS = {
  PLAY: "overlay:play",
  STOP: "overlay:stop",
  HEARTBEAT: "overlay:heartbeat",
  ERROR: "overlay:error",
  PLAYBACK_STATE: "overlay:playback-state",
  MEME_TRIGGER: "overlay:meme-trigger",
};
