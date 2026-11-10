export function createBoardLegacyWireupRegistrationUtils() {
    function registerBoardInteractions(params) {
        params.wireBoardInteractions({
            bindKeyboardHandler: params.bindKeyboardHandler,
            resolveDialogKeyboardAction: (key) => params.resolveDialogKeyboardAction({
                key,
                hasAddDialog: !!params.state.resolveAddDialog,
                hasRenameDialog: !!params.state.resolveRenameDialog,
                hasDeleteDialog: !!params.state.resolveDeleteDialog,
            }),
            closeAddDialog: () => {
                params.closeAddDialog(null);
            },
            closeRenameDialog: () => {
                params.closeRenameDialog(null);
            },
            closeDeleteDialogCancel: () => {
                params.closeDeleteDialog(false);
            },
            closeDeleteDialogConfirm: () => {
                params.closeDeleteDialog(true);
            },
            isCaptureActive: () => !!params.state.captureItemId,
            resolveCaptureKeyInput: (event) => params.resolveCaptureKeyInput({
                key: event.key,
                accelerator: params.keyEventToAccelerator(event),
                display: params.keyEventToDisplay(event),
                currentPendingAccelerator: params.state.capturePendingAccelerator,
                currentPendingDisplay: params.state.capturePendingDisplay,
                formatAcceleratorDisplay: params.formatAcceleratorDisplay,
            }),
            onCaptureCancelled: () => {
                params.endCapture();
                params.clearStatus();
            },
            onCaptureUpdated: ({ nextPendingAccelerator, nextPendingDisplay, statusMessage, }) => {
                params.state.capturePendingAccelerator = nextPendingAccelerator;
                params.state.capturePendingDisplay = nextPendingDisplay;
                params.refreshCaptureUi();
                params.setStatusSuccess(statusMessage);
            },
            bindLifecycleHandlers: params.bindLifecycleHandlers,
            flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
            schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
            bindCaptureUi: params.bindCaptureUi,
            isCaptureUiReady: params.isCaptureUiReady(),
            captureCancelButton: params.nodes.captureCancelButton,
            captureSaveButton: params.nodes.captureSaveButton,
            onCancelCapture: () => {
                params.endCapture();
                params.clearStatus();
            },
            onSaveCapture: () => {
                void params.commitCapturedShortcut();
            },
            refreshCaptureUi: params.refreshCaptureUi,
            bindAddDialog: params.bindAddDialog,
            isAddDialogReady: params.isAddDialogReady(),
            addForm: params.nodes.addForm,
            addUrlInput: params.nodes.addUrlInput,
            addTitleInput: params.nodes.addTitleInput,
            addMessageInput: params.nodes.addMessageInput,
            addRefreshInput: params.nodes.addRefreshInput,
            addCancelButton: params.nodes.addCancelButton,
            addOverlay: params.nodes.addOverlay,
            isHttpUrl: params.isHttpUrl,
            setStatusError: params.setStatusError,
            closeAddDialogWithValue: params.closeAddDialog,
            bindBackdropClose: params.bindBackdropClose,
            bindAddLinkButton: params.bindAddLinkButton,
            addLinkButton: params.nodes.addLinkButton,
            openAddDialog: params.openAddDialog,
            addItemFromLink: params.addItemFromLink,
            bindRenameDialog: params.bindRenameDialog,
            isRenameDialogReady: params.isRenameDialogReady(),
            renameForm: params.nodes.renameForm,
            renameInput: params.nodes.renameInput,
            renameMessageInput: params.nodes.renameMessageInput,
            renameCancelButton: params.nodes.renameCancelButton,
            renameOverlay: params.nodes.renameOverlay,
            closeRenameDialogWithValue: params.closeRenameDialog,
            bindDeleteDialog: params.bindDeleteDialog,
            isDeleteDialogReady: params.isDeleteDialogReady(),
            deleteCancelButton: params.nodes.deleteCancelButton,
            deleteConfirmButton: params.nodes.deleteConfirmButton,
            deleteOverlay: params.nodes.deleteOverlay,
            closeDeleteDialogWithValue: params.closeDeleteDialog,
            bindSearchControls: params.bindSearchControls,
            searchForm: params.nodes.searchForm,
            searchInput: params.nodes.searchInput,
            refreshButton: params.nodes.refreshButton,
            stopPlaybackButton: params.nodes.stopPlaybackButton,
            runSearchNow: params.runSearchNow,
            scheduleInstantSearch: params.scheduleInstantSearch,
            flushPreviewMessageChanges: params.flushPreviewMessageChanges,
            loadItemsAndRender: params.loadItemsAndRender,
            stopCurrentPlayback: params.stopCurrentPlayback,
        });
    }
    return {
        registerBoardInteractions,
    };
}
