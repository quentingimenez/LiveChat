import type { MediaFrameNodes } from './legacy-media-frame-utils';

export interface OverlayLegacyMediaFrameLayoutState {
  activeMediaFrame: HTMLElement | null;
  activeMediaContent: HTMLElement | null;
  activeMediaHeaderLeft: HTMLElement | null;
  activeMediaHeaderRight: HTMLElement | null;
  activeMediaFooter: HTMLElement | null;
  mediaHeaderLayoutRaf: number | null;
}

interface HeaderShift {
  leftShiftPx: number;
  rightShiftPx: number;
}

interface ClearMediaHeaderLayoutRafParams {
  state: OverlayLegacyMediaFrameLayoutState;
  cancelAnimationFrame: ((handle: number) => void) | null;
  clearTimeout: (handle: number) => void;
}

interface SyncMediaHeaderLayoutParams {
  state: OverlayLegacyMediaFrameLayoutState;
  computeHeaderShift: (params: {
    mediaWidth: number;
    leftWidth: number;
    rightWidth: number;
    minGapPx?: number;
  }) => HeaderShift;
}

interface ScheduleMediaHeaderLayoutParams {
  state: OverlayLegacyMediaFrameLayoutState;
  requestAnimationFrame: ((callback: () => void) => number) | null;
  setTimeout: (callback: () => void, delayMs: number) => number;
  syncMediaHeaderLayout: () => void;
}

interface EnsureMediaFrameParams {
  mediaLayer: unknown;
  countdownLayer: unknown;
  state: OverlayLegacyMediaFrameLayoutState;
  ensureCountdownLayerParent: (countdownLayer: unknown, headerRight: unknown) => void;
  createMediaFrameNodes: (params: { countdownLayer?: unknown }) => MediaFrameNodes;
  scheduleMediaHeaderLayout: () => void;
}

interface ResetMediaFrameStateParams {
  state: OverlayLegacyMediaFrameLayoutState;
}

export interface OverlayLegacyMediaFrameLayoutUtils {
  clearMediaHeaderLayoutRaf(params: ClearMediaHeaderLayoutRafParams): void;
  syncMediaHeaderLayout(params: SyncMediaHeaderLayoutParams): void;
  scheduleMediaHeaderLayout(params: ScheduleMediaHeaderLayoutParams): void;
  ensureMediaFrame(params: EnsureMediaFrameParams): HTMLElement | null;
  resetMediaFrameState(params: ResetMediaFrameStateParams): void;
}

export function createOverlayLegacyMediaFrameLayoutUtils(): OverlayLegacyMediaFrameLayoutUtils {
  function clearMediaHeaderLayoutRaf(params: ClearMediaHeaderLayoutRafParams): void {
    if (params.state.mediaHeaderLayoutRaf === null) {
      return;
    }

    if (typeof params.cancelAnimationFrame === 'function') {
      params.cancelAnimationFrame(params.state.mediaHeaderLayoutRaf);
    } else {
      params.clearTimeout(params.state.mediaHeaderLayoutRaf);
    }

    params.state.mediaHeaderLayoutRaf = null;
  }

  function syncMediaHeaderLayout(params: SyncMediaHeaderLayoutParams): void {
    if (
      !(params.state.activeMediaFrame instanceof HTMLElement) ||
      !(params.state.activeMediaHeaderLeft instanceof HTMLElement) ||
      !(params.state.activeMediaHeaderRight instanceof HTMLElement)
    ) {
      return;
    }

    const mediaNode = params.state.activeMediaContent?.firstElementChild;
    const mediaWidth =
      mediaNode instanceof HTMLElement
        ? Math.max(0, Math.round(mediaNode.getBoundingClientRect().width))
        : Math.max(0, Math.round(params.state.activeMediaFrame.getBoundingClientRect().width));

    const shift = params.computeHeaderShift({
      mediaWidth,
      leftWidth: params.state.activeMediaHeaderLeft.scrollWidth,
      rightWidth: params.state.activeMediaHeaderRight.scrollWidth,
      minGapPx: 14,
    });

    params.state.activeMediaFrame.style.setProperty('--header-left-shift', `${shift.leftShiftPx}px`);
    params.state.activeMediaFrame.style.setProperty('--header-right-shift', `${shift.rightShiftPx}px`);
  }

  function scheduleMediaHeaderLayout(params: ScheduleMediaHeaderLayoutParams): void {
    if (params.state.mediaHeaderLayoutRaf !== null) {
      return;
    }

    if (typeof params.requestAnimationFrame === 'function') {
      params.state.mediaHeaderLayoutRaf = params.requestAnimationFrame(() => {
        params.state.mediaHeaderLayoutRaf = null;
        params.syncMediaHeaderLayout();
      });
      return;
    }

    params.state.mediaHeaderLayoutRaf = params.setTimeout(() => {
      params.state.mediaHeaderLayoutRaf = null;
      params.syncMediaHeaderLayout();
    }, 0);
  }

  function ensureMediaFrame(params: EnsureMediaFrameParams): HTMLElement | null {
    if (!(params.mediaLayer instanceof HTMLElement)) {
      return null;
    }

    if (
      params.state.activeMediaFrame instanceof HTMLElement &&
      params.state.activeMediaContent instanceof HTMLElement &&
      params.state.activeMediaHeaderLeft instanceof HTMLElement &&
      params.state.activeMediaHeaderRight instanceof HTMLElement
    ) {
      params.ensureCountdownLayerParent(params.countdownLayer, params.state.activeMediaHeaderRight);
      return params.state.activeMediaContent;
    }

    const nextMediaFrameNodes = params.createMediaFrameNodes({
      countdownLayer: params.countdownLayer,
    });
    params.mediaLayer.appendChild(nextMediaFrameNodes.frame);

    params.state.activeMediaFrame = nextMediaFrameNodes.frame;
    params.state.activeMediaContent = nextMediaFrameNodes.content;
    params.state.activeMediaHeaderLeft = nextMediaFrameNodes.headerLeft;
    params.state.activeMediaHeaderRight = nextMediaFrameNodes.headerRight;
    params.state.activeMediaFooter = nextMediaFrameNodes.footer;

    params.scheduleMediaHeaderLayout();
    return nextMediaFrameNodes.content;
  }

  function resetMediaFrameState(params: ResetMediaFrameStateParams): void {
    params.state.activeMediaFrame = null;
    params.state.activeMediaContent = null;
    params.state.activeMediaHeaderLeft = null;
    params.state.activeMediaHeaderRight = null;
    params.state.activeMediaFooter = null;
    params.state.mediaHeaderLayoutRaf = null;
  }

  return {
    clearMediaHeaderLayoutRaf,
    syncMediaHeaderLayout,
    scheduleMediaHeaderLayout,
    ensureMediaFrame,
    resetMediaFrameState,
  };
}
