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
