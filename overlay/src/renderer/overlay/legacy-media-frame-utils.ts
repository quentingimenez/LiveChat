interface ComputeHeaderShiftParams {
  mediaWidth: number;
  leftWidth: number;
  rightWidth: number;
  minGapPx?: number;
}

interface HeaderShift {
  leftShiftPx: number;
  rightShiftPx: number;
}

interface CreateMediaFrameNodesParams {
  countdownLayer?: unknown;
}

export interface MediaFrameNodes {
  frame: HTMLDivElement;
  content: HTMLDivElement;
  headerLeft: HTMLDivElement;
  headerRight: HTMLDivElement;
  footer: HTMLDivElement;
}

export interface OverlayLegacyMediaFrameUtils {
  computeHeaderShift(params: ComputeHeaderShiftParams): HeaderShift;
  createMediaFrameNodes(params: CreateMediaFrameNodesParams): MediaFrameNodes;
  ensureCountdownLayerParent(countdownLayer: unknown, headerRight: unknown): void;
}

function toSafeNumber(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return 0;
  }

  return value;
}

export function createOverlayLegacyMediaFrameUtils(): OverlayLegacyMediaFrameUtils {
  function computeHeaderShift(params: ComputeHeaderShiftParams): HeaderShift {
    const mediaWidth = Math.max(0, Math.round(toSafeNumber(params.mediaWidth)));
    const leftWidth = Math.max(0, Math.round(toSafeNumber(params.leftWidth)));
    const rightWidth = Math.max(0, Math.round(toSafeNumber(params.rightWidth)));
    const minGapPx = Math.max(0, Math.round(toSafeNumber(params.minGapPx ?? 14)));
    const overflowPx = mediaWidth > 0 ? leftWidth + rightWidth + minGapPx - mediaWidth : 0;

    if (overflowPx <= 0) {
      return {
        leftShiftPx: 0,
        rightShiftPx: 0
      };
    }

    return {
      leftShiftPx: Math.ceil(overflowPx / 2),
      rightShiftPx: Math.floor(overflowPx / 2)
    };
  }

  function createMediaFrameNodes(params: CreateMediaFrameNodesParams): MediaFrameNodes {
    const frame = document.createElement('div');
    frame.className = 'overlay-media-frame';
    frame.style.setProperty('--header-left-shift', '0px');
    frame.style.setProperty('--header-right-shift', '0px');

    const header = document.createElement('div');
    header.className = 'overlay-media-header';

    const headerLeft = document.createElement('div');
    headerLeft.className = 'overlay-media-header-left';

    const headerRight = document.createElement('div');
    headerRight.className = 'overlay-media-header-right';
    header.appendChild(headerLeft);
    header.appendChild(headerRight);

    const content = document.createElement('div');
    content.className = 'overlay-media-content';

    const footer = document.createElement('div');
    footer.className = 'overlay-media-footer';
    footer.style.display = 'none';

    frame.appendChild(header);
    frame.appendChild(content);
    frame.appendChild(footer);

    ensureCountdownLayerParent(params.countdownLayer, headerRight);

    return {
      frame,
      content,
      headerLeft,
      headerRight,
      footer
    };
  }

  function ensureCountdownLayerParent(countdownLayer: unknown, headerRight: unknown): void {
    if (!(countdownLayer instanceof HTMLElement)) {
      return;
    }

    if (!(headerRight instanceof HTMLElement)) {
      return;
    }

    if (countdownLayer.parentElement !== headerRight) {
      headerRight.appendChild(countdownLayer);
    }
  }

  return {
    computeHeaderShift,
    createMediaFrameNodes,
    ensureCountdownLayerParent
  };
}
