interface ResetOverlayLayersParams {
  mediaLayer: unknown;
  textLayer: unknown;
  overlayRoot: unknown;
  countdownLayer: unknown;
}

export interface OverlayLegacyOverlayResetUtils {
  releaseObjectUrl(activeObjectUrl: unknown): null;
  resetOverlayLayers(params: ResetOverlayLayersParams): void;
}

export function createOverlayLegacyOverlayResetUtils(): OverlayLegacyOverlayResetUtils {
  function releaseObjectUrl(activeObjectUrl: unknown): null {
    if (typeof activeObjectUrl === 'string' && activeObjectUrl) {
      URL.revokeObjectURL(activeObjectUrl);
    }

    return null;
  }

  function resetOverlayLayers(params: ResetOverlayLayersParams): void {
    if (params.mediaLayer instanceof HTMLElement) {
      params.mediaLayer.innerHTML = '';
    }

    if (params.textLayer instanceof HTMLElement) {
      params.textLayer.innerHTML = '';
      params.textLayer.style.display = 'none';
    }

    if (
      params.overlayRoot instanceof HTMLElement &&
      params.countdownLayer instanceof HTMLElement &&
      params.countdownLayer.parentElement !== params.overlayRoot
    ) {
      params.overlayRoot.appendChild(params.countdownLayer);
    }
  }

  return {
    releaseObjectUrl,
    resetOverlayLayers
  };
}
