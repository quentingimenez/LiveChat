function toMediaKind(kind) {
    const normalized = `${kind || ''}`.trim().toLowerCase();
    if (normalized === 'image' || normalized === 'audio' || normalized === 'video') {
        return normalized;
    }
    return 'video';
}
function isFiniteNumber(value) {
    return typeof value === 'number' && Number.isFinite(value);
}
export function createOverlayLegacyMediaDomUtils() {
    function createMediaElement(kind) {
        const mediaKind = toMediaKind(kind);
        if (mediaKind === 'image') {
            return document.createElement('img');
        }
        if (mediaKind === 'audio') {
            const audio = document.createElement('audio');
            audio.autoplay = true;
            audio.controls = false;
            audio.preload = 'auto';
            return audio;
        }
        const video = document.createElement('video');
        video.autoplay = true;
        video.controls = false;
        video.playsInline = true;
        video.preload = 'auto';
        return video;
    }
    function attachMediaElement(params) {
        const targetContainer = params.targetContainer;
        const element = params.element;
        if (!(targetContainer instanceof HTMLElement)) {
            return false;
        }
        if (!(element instanceof HTMLElement)) {
            return false;
        }
        const mediaKind = toMediaKind(params.mediaKind);
        targetContainer.appendChild(element);
        if (mediaKind === 'image') {
            element.addEventListener('load', () => {
                params.onMediaReady();
            }, { once: true });
            return true;
        }
        element.addEventListener('loadedmetadata', () => {
            params.onMediaReady();
        });
        return true;
    }
    function applyMediaVolume(params) {
        const root = params.root;
        if (!(root instanceof HTMLElement)) {
            return;
        }
        const mediaElements = root.querySelectorAll('audio,video');
        mediaElements.forEach((element) => {
            if (!(element instanceof HTMLMediaElement)) {
                return;
            }
            const forceMuted = element.dataset?.forceMuted === '1';
            if (forceMuted) {
                element.volume = 0;
                element.muted = true;
                return;
            }
            const gain = isFiniteNumber(params.perceptualGain) ? params.perceptualGain : 1;
            element.volume = gain;
            element.muted = false;
        });
    }
    function buildAuthorizedMediaUrl(rawUrl, clientToken) {
        // Protocole v2 : les URLs média sont déjà signées par le serveur
        // (`?clientId&exp&sig[&startOffsetSec]`). Le renderer doit les utiliser
        // VERBATIM : ne jamais ajouter `?token=` (cf. docs/rust/overlay-bridge.md
        // §8.4 et docs/rust/protocol-v2.md §6.2). `clientToken` reste accepté
        // pour ne pas modifier la signature des appelants legacy.
        void clientToken;
        return `${rawUrl || ''}`;
    }
    return {
        createMediaElement,
        attachMediaElement,
        applyMediaVolume,
        buildAuthorizedMediaUrl
    };
}
