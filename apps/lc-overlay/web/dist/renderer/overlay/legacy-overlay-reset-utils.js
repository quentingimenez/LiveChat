export function createOverlayLegacyOverlayResetUtils() {
    function releaseObjectUrl(activeObjectUrl) {
        if (typeof activeObjectUrl === 'string' && activeObjectUrl) {
            URL.revokeObjectURL(activeObjectUrl);
        }
        return null;
    }
    // A media element removed from the DOM keeps decoding (and keeps its audio
    // playing) until it is explicitly paused and its source is dropped. The
    // Windows media session also stays on the player as long as a source is
    // attached, so a stop must release every media element before the layers
    // are cleared.
    function releaseMediaElement(element) {
        if (!element || typeof element.pause !== 'function') {
            return;
        }
        try {
            element.pause();
        }
        catch {
            // A detached or already-closed element cannot be paused: nothing
            // else to release.
        }
        if (typeof element.removeAttribute === 'function') {
            for (const source of Array.from(element.querySelectorAll ? element.querySelectorAll('source') : [])) {
                source.remove();
            }
            element.removeAttribute('src');
        }
        if (typeof element.load === 'function') {
            try {
                element.load();
            }
            catch {
                // Loading is best effort: the element is about to be removed.
            }
        }
    }
    function releaseMediaElements(root) {
        if (!root || typeof root.querySelectorAll !== 'function') {
            return;
        }
        for (const element of Array.from(root.querySelectorAll('video, audio'))) {
            releaseMediaElement(element);
        }
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
        releaseMediaElement,
        releaseMediaElements,
        resetOverlayLayers
    };
}
