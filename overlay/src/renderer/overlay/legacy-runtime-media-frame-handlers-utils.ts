import type { OverlayLegacyMediaFrameLayoutUtils } from './legacy-media-frame-layout-utils.js';
import type { OverlayLegacyMediaFrameUtils } from './legacy-media-frame-utils.js';
import type { OverlayRuntimeNodes, OverlayRuntimeState } from './legacy-runtime-types.js';

interface CreateRuntimeMediaFrameHandlersParams {
  nodes: OverlayRuntimeNodes;
  state: OverlayRuntimeState;
  mediaFrameUtils: OverlayLegacyMediaFrameUtils;
  mediaFrameLayoutUtils: OverlayLegacyMediaFrameLayoutUtils;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearTimeout: (timeoutId: number) => void;
  requestAnimationFrame: ((callback: () => void) => number) | null;
  cancelAnimationFrame: ((id: number) => void) | null;
}

interface RuntimeMediaFrameHandlers {
  clearMediaHeaderLayoutRaf: () => void;
  scheduleMediaHeaderLayout: () => void;
  ensureMediaFrame: () => HTMLElement | null;
  resetMediaFrameState: () => void;
}

export interface OverlayLegacyRuntimeMediaFrameHandlersUtils {
  createRuntimeMediaFrameHandlers(params: CreateRuntimeMediaFrameHandlersParams): RuntimeMediaFrameHandlers;
}

export function createOverlayLegacyRuntimeMediaFrameHandlersUtils(): OverlayLegacyRuntimeMediaFrameHandlersUtils {
  function createRuntimeMediaFrameHandlers(params: CreateRuntimeMediaFrameHandlersParams): RuntimeMediaFrameHandlers {
    function clearMediaHeaderLayoutRaf(): void {
      params.mediaFrameLayoutUtils.clearMediaHeaderLayoutRaf({
        state: params.state.mediaFrameState,
        cancelAnimationFrame: params.cancelAnimationFrame,
        clearTimeout: params.clearTimeout,
      });
    }

    function syncMediaHeaderLayout(): void {
      params.mediaFrameLayoutUtils.syncMediaHeaderLayout({
        state: params.state.mediaFrameState,
        computeHeaderShift: params.mediaFrameUtils.computeHeaderShift,
      });
    }

    function scheduleMediaHeaderLayout(): void {
      params.mediaFrameLayoutUtils.scheduleMediaHeaderLayout({
        state: params.state.mediaFrameState,
        requestAnimationFrame: params.requestAnimationFrame,
        setTimeout: params.setTimeout,
        syncMediaHeaderLayout,
      });
    }

    function ensureMediaFrame(): HTMLElement | null {
      return params.mediaFrameLayoutUtils.ensureMediaFrame({
        mediaLayer: params.nodes.mediaLayer,
        countdownLayer: params.nodes.countdownLayer,
        state: params.state.mediaFrameState,
        ensureCountdownLayerParent: params.mediaFrameUtils.ensureCountdownLayerParent,
        createMediaFrameNodes: params.mediaFrameUtils.createMediaFrameNodes,
        scheduleMediaHeaderLayout,
      });
    }

    function resetMediaFrameState(): void {
      params.mediaFrameLayoutUtils.resetMediaFrameState({
        state: params.state.mediaFrameState,
      });
    }

    return {
      clearMediaHeaderLayoutRaf,
      scheduleMediaHeaderLayout,
      ensureMediaFrame,
      resetMediaFrameState,
    };
  }

  return {
    createRuntimeMediaFrameHandlers,
  };
}
