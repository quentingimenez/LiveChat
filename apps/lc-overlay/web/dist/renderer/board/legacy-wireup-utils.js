export function createBoardLegacyWireupUtils() {
    function wireBoardInteractions(params) {
        params.bindKeyboardHandler({
            resolveDialogKeyboardAction: params.resolveDialogKeyboardAction,
            closeAddDialog: params.closeAddDialog,
            closeRenameDialog: params.closeRenameDialog,
            closeDeleteDialogCancel: params.closeDeleteDialogCancel,
            closeDeleteDialogConfirm: params.closeDeleteDialogConfirm,
            isCaptureActive: params.isCaptureActive,
            resolveCaptureKeyInput: params.resolveCaptureKeyInput,
            onCaptureCancelled: params.onCaptureCancelled,
            onCaptureUpdated: params.onCaptureUpdated,
        });
        params.bindLifecycleHandlers({
            flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
            schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
        });
        params.bindCaptureUi({
            isCaptureUiReady: params.isCaptureUiReady,
            captureCancelButton: params.captureCancelButton,
            captureSaveButton: params.captureSaveButton,
            onCancelCapture: params.onCancelCapture,
            onSaveCapture: params.onSaveCapture,
            refreshCaptureUi: params.refreshCaptureUi,
        });
        params.bindAddDialog({
            isAddDialogReady: params.isAddDialogReady,
            addForm: params.addForm,
            addUrlInput: params.addUrlInput,
            addTitleInput: params.addTitleInput,
            addMessageInput: params.addMessageInput,
            addRefreshInput: params.addRefreshInput,
            addCancelButton: params.addCancelButton,
            addOverlay: params.addOverlay,
            isHttpUrl: params.isHttpUrl,
            setStatusError: params.setStatusError,
            closeAddDialog: params.closeAddDialogWithValue,
            bindBackdropClose: params.bindBackdropClose,
        });
        params.bindAddLinkButton({
            addLinkButton: params.addLinkButton,
            onOpenAndSubmit: () => {
                void (async () => {
                    const payload = await params.openAddDialog();
                    if (!payload) {
                        return;
                    }
                    await params.addItemFromLink(payload);
                })();
            },
        });
        params.bindRenameDialog({
            isRenameDialogReady: params.isRenameDialogReady,
            renameForm: params.renameForm,
            renameInput: params.renameInput,
            renameMessageInput: params.renameMessageInput,
            renameCancelButton: params.renameCancelButton,
            renameOverlay: params.renameOverlay,
            closeRenameDialog: params.closeRenameDialogWithValue,
            bindBackdropClose: params.bindBackdropClose,
        });
        params.bindDeleteDialog({
            isDeleteDialogReady: params.isDeleteDialogReady,
            deleteCancelButton: params.deleteCancelButton,
            deleteConfirmButton: params.deleteConfirmButton,
            deleteOverlay: params.deleteOverlay,
            closeDeleteDialog: params.closeDeleteDialogWithValue,
            bindBackdropClose: params.bindBackdropClose,
        });
        params.bindSearchControls({
            searchForm: params.searchForm,
            searchInput: params.searchInput,
            refreshButton: params.refreshButton,
            stopPlaybackButton: params.stopPlaybackButton,
            onSearchSubmit: () => {
                void params.runSearchNow();
            },
            onSearchInput: () => {
                params.scheduleInstantSearch();
            },
            onRefresh: () => {
                void (async () => {
                    const flushed = await params.flushPreviewMessageChanges();
                    if (!flushed) {
                        return;
                    }
                    await params.loadItemsAndRender();
                })();
            },
            onStopPlayback: () => {
                void params.stopCurrentPlayback();
            },
        });
    }
    return {
        wireBoardInteractions,
    };
}
