export function createBoardLegacyCaptureOrchestrationUtils() {
    function createCaptureHandlers(params) {
        function refreshCaptureUi() {
            params.captureFlowUtils.refreshCaptureUi({
                isCaptureUiReady: params.isCaptureUiReady(),
                captureCurrentNode: params.captureCurrentNode,
                captureSaveButton: params.captureSaveButton,
                state: params.getCaptureState(),
            });
        }
        function endCapture() {
            params.captureFlowUtils.endCapture({
                captureOverlay: params.captureOverlay,
                setCaptureState: params.setCaptureState,
                refreshCaptureUi,
            });
        }
        async function commitCapturedShortcut() {
            await params.captureFlowUtils.commitCapturedShortcut({
                state: params.getCaptureState(),
                bindings: params.getBindings(),
                assignShortcutForItem: params.assignShortcutForItem,
                persistBindings: params.persistBindings,
                endCapture,
                renderList: params.renderList,
                formatAcceleratorDisplay: params.formatAcceleratorDisplay,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: (message) => {
                    params.setStatusError(message || "Impossible d'assigner ce raccourci.");
                },
            });
        }
        function beginCaptureForItem(itemId) {
            params.captureFlowUtils.beginCaptureForItem({
                itemId,
                captureOverlay: params.captureOverlay,
                setCaptureState: params.setCaptureState,
                refreshCaptureUi,
                setStatusSuccess: params.setStatusSuccess,
            });
        }
        return {
            refreshCaptureUi,
            endCapture,
            commitCapturedShortcut,
            beginCaptureForItem,
        };
    }
    return {
        createCaptureHandlers,
    };
}
