import { VOLUME_CURVE_GAMMA } from './legacy-utils.js';
function queryNode(documentRef, id) {
    return documentRef.getElementById(id);
}
export function createOverlayLegacyRuntimeInputsUtils() {
    function createRuntimeInputs(documentRef) {
        const nodes = {
            overlayRoot: queryNode(documentRef, 'overlay-root'),
            mediaLayer: queryNode(documentRef, 'media-layer'),
            countdownLayer: queryNode(documentRef, 'countdown-layer'),
            textLayer: queryNode(documentRef, 'text-layer'),
        };
        const state = {
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
            activeVolume: null,
            // Re-fits a scaled media after a window resize (display resolution
            // change); set by the render handlers for every media played.
            activeMediaRelayout: null,
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
                volumeCurveGamma: VOLUME_CURVE_GAMMA,
            },
        };
    }
    return {
        createRuntimeInputs,
    };
}
