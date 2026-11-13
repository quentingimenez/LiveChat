export function createOverlayLegacyOverlayResetUtils() {
    function releaseObjectUrl(activeObjectUrl) {
        if (typeof activeObjectUrl === 'string' && activeObjectUrl) {
            URL.revokeObjectURL(activeObjectUrl);
        }
        return null;
    }
    function resetOverlayLayers(params) {
        if (params.mediaLayer instanceof HTMLElement) {
            params.mediaLayer.innerHTML = '';
        }
        if (params.textLayer instanceof HTMLElement) {
            params.textLayer.innerHTML = '';
            params.textLayer.style.display = 'none';
        }
        if (params.overlayRoot instanceof HTMLElement &&
            params.countdownLayer instanceof HTMLElement &&
            params.countdownLayer.parentElement !== params.overlayRoot) {
            params.overlayRoot.appendChild(params.countdownLayer);
        }
    }
    return {
        releaseObjectUrl,
        resetOverlayLayers
    };
}
