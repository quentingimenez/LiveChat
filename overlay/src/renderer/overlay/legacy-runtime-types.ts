import type { OverlayLegacyMediaFrameLayoutState } from './legacy-media-frame-layout-utils';

export interface OverlayRuntimeNodes {
  overlayRoot: HTMLElement | null;
  mediaLayer: HTMLElement | null;
  countdownLayer: HTMLElement | null;
  textLayer: HTMLElement | null;
}

export interface OverlayRuntimeState {
  overlayConfig: Record<string, unknown>;
  resetTimer: number | null;
  countdownState: {
    timer: number | null;
    remainingMs: number | null;
    lastTickAtMs: number | null;
    paused: boolean;
    autoClear: boolean;
  };
  playbackSyncTimer: number | null;
  activePlayableElement: HTMLMediaElement | null;
  activePlayback: {
    jobId: string;
    state: string;
  } | null;
  activeObjectUrl: string | null;
  mediaFrameState: OverlayLegacyMediaFrameLayoutState;
}

export interface OverlayRuntimeConstants {
  volumeCurveGamma: number;
}

export interface OverlayRuntimeInputs {
  nodes: OverlayRuntimeNodes;
  state: OverlayRuntimeState;
  constants: OverlayRuntimeConstants;
}
