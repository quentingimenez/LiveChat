export const MediaAssetKind = {
  IMAGE: 'IMAGE',
  AUDIO: 'AUDIO',
  VIDEO: 'VIDEO',
} as const;

export type MediaAssetKind = (typeof MediaAssetKind)[keyof typeof MediaAssetKind];

export const MediaAssetStatus = {
  PROCESSING: 'PROCESSING',
  READY: 'READY',
  FAILED: 'FAILED',
} as const;

export type MediaAssetStatus = (typeof MediaAssetStatus)[keyof typeof MediaAssetStatus];

export const PlaybackJobStatus = {
  PENDING: 'PENDING',
  PLAYING: 'PLAYING',
  DONE: 'DONE',
  FAILED: 'FAILED',
} as const;

export type PlaybackJobStatus = (typeof PlaybackJobStatus)[keyof typeof PlaybackJobStatus];
