function requireUtility(utility, utilityName) {
    if (utility === undefined || utility === null) {
        throw new Error(`Board legacy utility unavailable: ${utilityName}`);
    }
    return utility;
}
export function createBoardLegacyRuntimeCaptureHandlersUtils() {
    function createRuntimeCaptureHandlers(params) {
        const captureFlowUtils = requireUtility(window.__boardLegacyCaptureFlowUtils, 'capture-flow');
        const captureOrchestrationUtils = requireUtility(window.__boardLegacyCaptureOrchestrationUtils, 'capture-orchestration');
        const bindingsUtils = requireUtility(window.__boardLegacyBindingsUtils, 'bindings');
        const shortcutUtils = requireUtility(window.__boardLegacyShortcutUtils, 'shortcut');
        const formatAcceleratorDisplay = (accelerator) => shortcutUtils.formatAcceleratorDisplay(accelerator);
        const captureHandlers = captureOrchestrationUtils.createCaptureHandlers({
            captureFlowUtils,
            isCaptureUiReady: params.isCaptureUiReady,
            captureCurrentNode: params.nodes.captureCurrentNode,
            captureSaveButton: params.nodes.captureSaveButton,
            captureOverlay: params.nodes.captureOverlay,
            getCaptureState: () => ({
                captureItemId: params.state.captureItemId,
                capturePendingAccelerator: params.state.capturePendingAccelerator,
                capturePendingDisplay: params.state.capturePendingDisplay,
            }),
            setCaptureState: ({ captureItemId, capturePendingAccelerator, capturePendingDisplay }) => {
                params.state.captureItemId = captureItemId;
                params.state.capturePendingAccelerator = capturePendingAccelerator;
                params.state.capturePendingDisplay = capturePendingDisplay;
            },
            getBindings: () => params.state.bindings,
            assignShortcutForItem: bindingsUtils.assignShortcutForItem,
            persistBindings: params.persistBindings,
            renderList: params.renderList,
            formatAcceleratorDisplay,
            setStatusSuccess: params.setStatusSuccess,
            setStatusError: params.setStatusError,
        });
        return {
            ...captureHandlers,
            formatAcceleratorDisplay,
            keyEventToDisplay: shortcutUtils.keyEventToDisplay,
            keyEventToAccelerator: shortcutUtils.keyEventToAccelerator,
        };
    }
    return {
        createRuntimeCaptureHandlers,
    };
}
