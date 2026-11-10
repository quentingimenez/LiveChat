export function createBoardLegacyRuntimeFoundationUtils() {
    function createRuntimeFoundation(params) {
        const statusController = params.statusUtils.createStatusController({
            statusNode: params.nodes.statusNode,
            setTimer: params.setTimeout,
            clearTimer: params.clearTimeout,
        });
        function setStatus(message, variant = 'success', options = {}) {
            statusController.setStatus(message, variant, options);
        }
        function clearStatus() {
            statusController.clearStatus();
        }
        function clearSearchDebounce() {
            params.state.searchDebounceTimeoutId = params.searchUtils.clearDebounceTimer(params.state.searchDebounceTimeoutId, params.clearTimeout);
        }
        function clearPreviewMessageAutosave() {
            if (params.state.previewMessageAutosaveTimeoutId === null) {
                return;
            }
            params.clearTimeout(params.state.previewMessageAutosaveTimeoutId);
            params.state.previewMessageAutosaveTimeoutId = null;
        }
        function fitPreviewMediaToStage() {
            params.previewLayoutUtils.fitPreviewMediaToStage({
                previewStageNode: params.nodes.previewStageNode,
                minHeightPx: params.previewMediaMinHeightPx,
                sideLayoutBreakpointPx: params.previewSideLayoutBreakpointPx,
            });
        }
        function schedulePreviewLayoutRefresh() {
            if (params.state.previewLayoutRafId !== null) {
                return;
            }
            params.state.previewLayoutRafId = params.requestAnimationFrame(() => {
                params.state.previewLayoutRafId = null;
                fitPreviewMediaToStage();
            });
        }
        function refreshPreviewLayoutObserverTargets() {
            params.previewLayoutUtils.refreshPreviewLayoutObserverTargets({
                previewLayoutObserver: params.state.previewLayoutObserver,
                previewStageNode: params.nodes.previewStageNode,
            });
        }
        function resetPreviewMessageEditor() {
            clearPreviewMessageAutosave();
            params.state.previewMessageEditor = null;
        }
        const runtimeContextHelpers = params.runtimeContextUtils.createRuntimeContextHelpers({
            boardUtils: params.boardUtils,
            previewMediaUtils: params.previewMediaUtils,
            selectionUtils: params.selectionUtils,
            bindingsUtils: params.bindingsUtils,
            dialogUtils: params.dialogUtils,
            state: params.state,
            previewStageNode: params.nodes.previewStageNode,
            volumeCurveGamma: params.volumeCurveGamma,
            captureOverlay: params.nodes.captureOverlay,
            captureCurrentNode: params.nodes.captureCurrentNode,
            captureCancelButton: params.nodes.captureCancelButton,
            captureSaveButton: params.nodes.captureSaveButton,
        });
        return {
            setStatus,
            clearStatus,
            clearSearchDebounce,
            clearPreviewMessageAutosave,
            schedulePreviewLayoutRefresh,
            refreshPreviewLayoutObserverTargets,
            resetPreviewMessageEditor,
            runtimeContextHelpers,
        };
    }
    return {
        createRuntimeFoundation,
    };
}
