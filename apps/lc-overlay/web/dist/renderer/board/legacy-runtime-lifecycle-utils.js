function requireUtility(utility, utilityName) {
    if (utility === undefined || utility === null) {
        throw new Error(`Board legacy utility unavailable: ${utilityName}`);
    }
    return utility;
}
export function createBoardLegacyRuntimeLifecycleUtils() {
    function registerWireupRuntime(params) {
        const wireupRuntimeUtils = requireUtility(window.__boardLegacyWireupRuntimeUtils, 'wireup-runtime');
        const wireupRegistrationUtils = requireUtility(window.__boardLegacyWireupRegistrationUtils, 'wireup-registration');
        const wireupUtils = requireUtility(window.__boardLegacyWireupUtils, 'wireup');
        const lifecycleEventUtils = requireUtility(window.__boardLegacyLifecycleEventUtils, 'lifecycle-events');
        const keyboardUtils = requireUtility(window.__boardLegacyKeyboardUtils, 'keyboard');
        const captureUtils = requireUtility(window.__boardLegacyCaptureUtils, 'capture');
        const eventBindingsUtils = requireUtility(window.__boardLegacyEventBindingsUtils, 'event-bindings');
        wireupRuntimeUtils.registerWireupRuntime({
            wireupRegistrationUtils,
            wireupUtils,
            lifecycleEventUtils,
            keyboardUtils,
            captureUtils,
            eventBindingsUtils,
            state: params.state,
            keyEventToAccelerator: params.keyEventToAccelerator,
            keyEventToDisplay: params.keyEventToDisplay,
            formatAcceleratorDisplay: params.formatAcceleratorDisplay,
            endCapture: params.endCapture,
            clearStatus: params.clearStatus,
            setStatusSuccess: params.setStatusSuccess,
            closeAddDialog: params.closeAddDialog,
            closeRenameDialog: params.closeRenameDialog,
            closeDeleteDialog: params.closeDeleteDialog,
            refreshCaptureUi: params.refreshCaptureUi,
            commitCapturedShortcut: params.commitCapturedShortcut,
            isCaptureUiReady: params.isCaptureUiReady,
            isAddDialogReady: params.isAddDialogReady,
            isRenameDialogReady: params.isRenameDialogReady,
            isDeleteDialogReady: params.isDeleteDialogReady,
            isHttpUrl: params.isHttpUrl,
            setStatusError: params.setStatusError,
            bindBackdropClose: params.bindBackdropClose,
            openAddDialog: params.openAddDialog,
            addItemFromLink: params.addItemFromLink,
            runSearchNow: params.runSearchNow,
            scheduleInstantSearch: params.scheduleInstantSearch,
            flushPreviewMessageChanges: params.flushPreviewMessageChanges,
            loadItemsAndRender: params.loadItemsAndRender,
            stopCurrentPlayback: params.stopCurrentPlayback,
            flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
            schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
            nodes: {
                captureCancelButton: params.nodes.captureCancelButton,
                captureSaveButton: params.nodes.captureSaveButton,
                addForm: params.nodes.addForm,
                addUrlInput: params.nodes.addUrlInput,
                addTitleInput: params.nodes.addTitleInput,
                addMessageInput: params.nodes.addMessageInput,
                addRefreshInput: params.nodes.addRefreshInput,
                addCancelButton: params.nodes.addCancelButton,
                addOverlay: params.nodes.addOverlay,
                addLinkButton: params.nodes.addLinkButton,
                renameForm: params.nodes.renameForm,
                renameInput: params.nodes.renameInput,
                renameMessageInput: params.nodes.renameMessageInput,
                renameCancelButton: params.nodes.renameCancelButton,
                renameOverlay: params.nodes.renameOverlay,
                deleteCancelButton: params.nodes.deleteCancelButton,
                deleteConfirmButton: params.nodes.deleteConfirmButton,
                deleteOverlay: params.nodes.deleteOverlay,
                searchForm: params.nodes.searchForm,
                searchInput: params.nodes.searchInput,
                refreshButton: params.nodes.refreshButton,
                stopPlaybackButton: params.nodes.stopPlaybackButton,
            },
        });
    }
    async function bootstrapRuntime(params) {
        const bootstrapOrchestrationUtils = requireUtility(window.__boardLegacyBootstrapOrchestrationUtils, 'bootstrap-orchestration');
        const bootstrapUtils = requireUtility(window.__boardLegacyBootstrapUtils, 'bootstrap');
        await bootstrapOrchestrationUtils.bootstrapBoardRuntime({
            bootstrapBoard: (nextParams) => bootstrapUtils.bootstrapBoard(nextParams),
            getConfig: params.getConfig,
            getMemeBindings: params.getMemeBindings,
            onSettings: params.onSettings,
            state: params.state,
            applyPreviewVolume: params.applyPreviewVolume,
            setStatusError: params.setStatusError,
            refreshButton: params.nodes.refreshButton,
            searchInput: params.nodes.searchInput,
            stopPlaybackButton: params.nodes.stopPlaybackButton,
            addLinkButton: params.nodes.addLinkButton,
            previewStageNode: params.nodes.previewStageNode,
            schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
            refreshPreviewLayoutObserverTargets: params.refreshPreviewLayoutObserverTargets,
            loadItemsAndRender: params.loadItemsAndRender,
        });
    }
    async function registerWireupAndBootstrap(params) {
        registerWireupRuntime(params);
        await bootstrapRuntime(params);
    }
    return {
        registerWireupRuntime,
        bootstrapRuntime,
        registerWireupAndBootstrap,
    };
}
