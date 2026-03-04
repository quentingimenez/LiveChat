import type { OverlayRuntimeInputs, OverlayRuntimeNodes, OverlayRuntimeState } from './legacy-runtime-types';

export interface OverlayLegacyRuntimeInputsUtils {
  createRuntimeInputs(documentRef: Document): OverlayRuntimeInputs;
}

function queryNode(documentRef: Document, id: string): HTMLElement | null {
  return documentRef.getElementById(id);
}

export function createOverlayLegacyRuntimeInputsUtils(): OverlayLegacyRuntimeInputsUtils {
  function createRuntimeInputs(documentRef: Document): OverlayRuntimeInputs {
    const nodes: OverlayRuntimeNodes = {
      overlayRoot: queryNode(documentRef, 'overlay-root'),
      mediaLayer: queryNode(documentRef, 'media-layer'),
      countdownLayer: queryNode(documentRef, 'countdown-layer'),
      textLayer: queryNode(documentRef, 'text-layer'),
    };

    const state: OverlayRuntimeState = {
      overlayConfig: {
        serverUrl: null,
        clientToken: null,
        guildId: null,
        clientId: null,
        volume: 1,
        showText: true,
      },
      resetTimer: null,
      countdownState: {
        timer: null,
        remainingMs: null,
        lastTickAtMs: null,
        paused: false,
        autoClear: false,
      },
      playbackSyncTimer: null,
      activePlayableElement: null,
      activePlayback: null,
      activeObjectUrl: null,
      mediaFrameState: {
        activeMediaFrame: null,
        activeMediaContent: null,
        activeMediaHeaderLeft: null,
        activeMediaHeaderRight: null,
        activeMediaFooter: null,
        mediaHeaderLayoutRaf: null,
      },
    };

    return {
      nodes,
      state,
      constants: {
        volumeCurveGamma: 2.2,
      },
    };
  }

  return {
    createRuntimeInputs,
  };
}
