import type {
  MemeBoardActionResponse,
  MemeBoardBindingsResponse,
  MemeBoardSetBindingsRequest,
  MemeBoardSetBindingsResponse,
  MemeBoardTriggerRequest
} from '../shared/meme-board';
import type {
  OverlayConfig,
  OverlayErrorPayload,
  OverlayPlayPayload,
  OverlayPlaybackStatePayload,
  OverlayPlaybackStopPayload,
  OverlaySettingsPayload,
  OverlayStopPayload
} from '../shared/overlay-config';
import type { PairingConsumeRequest, PairingConsumeResponse } from '../shared/pairing';

export interface LivechatOverlayApi {
  getConfig(): Promise<OverlayConfig>;
  rendererReady(): void;
  onPlay(callback: (payload: OverlayPlayPayload) => void): () => void;
  onStop(callback: (payload: OverlayStopPayload) => void): () => void;
  onSettings(callback: (payload: OverlaySettingsPayload) => void): () => void;
  reportError(payload: OverlayErrorPayload): void;
  reportPlaybackState(payload: OverlayPlaybackStatePayload): void;
  reportPlaybackStop(payload: OverlayPlaybackStopPayload): void;
  consumePairing(payload: PairingConsumeRequest): Promise<PairingConsumeResponse>;
  getMemeBindings(): Promise<MemeBoardBindingsResponse>;
  setMemeBindings(payload: MemeBoardSetBindingsRequest): Promise<MemeBoardSetBindingsResponse>;
  triggerMeme(payload: MemeBoardTriggerRequest): Promise<MemeBoardActionResponse>;
  stopMemePlayback(): Promise<MemeBoardActionResponse>;
}

declare global {
  interface Window {
    livechatOverlay: LivechatOverlayApi;
  }
}

export {};
