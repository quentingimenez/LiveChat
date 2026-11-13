export function createOverlayLegacyRuntimeMediaFrameHandlersUtils() {
    function createRuntimeMediaFrameHandlers(params) {
        function clearMediaHeaderLayoutRaf() {
            params.mediaFrameLayoutUtils.clearMediaHeaderLayoutRaf({
                state: params.state.mediaFrameState,
                cancelAnimationFrame: params.cancelAnimationFrame,
                clearTimeout: params.clearTimeout,
            });
        }
        function syncMediaHeaderLayout() {
            params.mediaFrameLayoutUtils.syncMediaHeaderLayout({
                state: params.state.mediaFrameState,
                computeHeaderShift: params.mediaFrameUtils.computeHeaderShift,
            });
        }
        function scheduleMediaHeaderLayout() {
            params.mediaFrameLayoutUtils.scheduleMediaHeaderLayout({
                state: params.state.mediaFrameState,
                requestAnimationFrame: params.requestAnimationFrame,
                setTimeout: params.setTimeout,
                syncMediaHeaderLayout,
            });
        }
        function ensureMediaFrame() {
            return params.mediaFrameLayoutUtils.ensureMediaFrame({
                mediaLayer: params.nodes.mediaLayer,
                countdownLayer: params.nodes.countdownLayer,
                state: params.state.mediaFrameState,
                ensureCountdownLayerParent: params.mediaFrameUtils.ensureCountdownLayerParent,
                createMediaFrameNodes: params.mediaFrameUtils.createMediaFrameNodes,
                scheduleMediaHeaderLayout,
            });
        }
        function resetMediaFrameState() {
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
