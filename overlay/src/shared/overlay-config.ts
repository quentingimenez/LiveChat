import type { Dictionary } from './types';

export interface OverlayConfig {
  serverUrl: string | null;
  clientToken: string | null;
  guildId: string | null;
  clientId: string | null;
  guestMode: boolean;
  showText: boolean;
  volume: number;
  memeBindings: Dictionary<string>;
}

export type OverlaySettingsPayload = OverlayConfig;

export interface OverlayPlaybackStatePayload {
  jobId: string;
  state: string;
  remainingMs: number | null;
}

export interface OverlayPlaybackStopPayload {
  jobId: string;
}

export interface OverlayErrorPayload {
  jobId: string;
  code: string;
  message: string;
}

export interface OverlayAuthorPayload {
  enabled?: boolean;
  name?: string;
  image?: string;
}

export interface OverlayTextPayload {
  enabled?: boolean;
  value?: string;
}

export interface OverlayMediaPayload {
  kind: 'image' | 'audio' | 'video';
  url?: string;
  sourceUrl?: string;
  mime?: string;
  startOffsetSec?: number;
  isVertical?: boolean;
}

export interface OverlayPlayPayload {
  jobId?: string;
  durationSec?: number;
  media?: OverlayMediaPayload;
  author?: OverlayAuthorPayload;
  text?: OverlayTextPayload;
  [key: string]: unknown;
}

export interface OverlayStopPayload {
  jobId?: string;
}
