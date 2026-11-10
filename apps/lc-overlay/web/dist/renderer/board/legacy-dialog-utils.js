export function createBoardLegacyDialogUtils() {
    function isAddDialogReady(nodes) {
        return (nodes.addLinkButton instanceof HTMLElement &&
            nodes.addOverlay instanceof HTMLElement &&
            nodes.addForm instanceof HTMLFormElement &&
            nodes.addUrlInput instanceof HTMLInputElement &&
            nodes.addTitleInput instanceof HTMLInputElement &&
            nodes.addMessageInput instanceof HTMLTextAreaElement &&
            nodes.addRefreshInput instanceof HTMLInputElement &&
            nodes.addCancelButton instanceof HTMLElement);
    }
    function isRenameDialogReady(nodes) {
        return (nodes.renameOverlay instanceof HTMLElement &&
            nodes.renameForm instanceof HTMLFormElement &&
            nodes.renameInput instanceof HTMLInputElement &&
            nodes.renameMessageInput instanceof HTMLTextAreaElement &&
            nodes.renameCancelButton instanceof HTMLElement);
    }
    function isDeleteDialogReady(nodes) {
        return (nodes.deleteOverlay instanceof HTMLElement &&
            nodes.deleteMessage instanceof HTMLElement &&
            nodes.deleteCancelButton instanceof HTMLElement &&
            nodes.deleteConfirmButton instanceof HTMLElement);
    }
    function isCaptureUiReady(nodes) {
        return (nodes.captureOverlay instanceof HTMLElement &&
            nodes.captureCurrentNode instanceof HTMLElement &&
            nodes.captureCancelButton instanceof HTMLElement &&
            nodes.captureSaveButton instanceof HTMLButtonElement);
    }
    function bindBackdropClose(overlayNode, closeDialog) {
        if (!(overlayNode instanceof HTMLElement) || typeof closeDialog !== 'function') {
            return;
        }
        let pointerStartedOnBackdrop = false;
        overlayNode.addEventListener('mousedown', (event) => {
            pointerStartedOnBackdrop = event.target === overlayNode;
        });
        overlayNode.addEventListener('click', (event) => {
            const clickedBackdrop = event.target === overlayNode;
            if (clickedBackdrop && pointerStartedOnBackdrop) {
                closeDialog();
            }
            pointerStartedOnBackdrop = false;
        });
    }
    return {
        isAddDialogReady,
        isRenameDialogReady,
        isDeleteDialogReady,
        isCaptureUiReady,
        bindBackdropClose
    };
}
