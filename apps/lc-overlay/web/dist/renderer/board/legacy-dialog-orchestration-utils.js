export function createBoardLegacyDialogOrchestrationUtils() {
    function createDialogHandlers(params) {
        function isAddDialogReady() {
            return params.dialogUtils.isAddDialogReady({
                addLinkButton: params.nodes.addLinkButton,
                addOverlay: params.nodes.addOverlay,
                addForm: params.nodes.addForm,
                addUrlInput: params.nodes.addUrlInput,
                addTitleInput: params.nodes.addTitleInput,
                addMessageInput: params.nodes.addMessageInput,
                addRefreshInput: params.nodes.addRefreshInput,
                addCancelButton: params.nodes.addCancelButton,
            });
        }
        function closeAddDialog(value) {
            params.dialogStateUtils.closeDialogWithResolver({
                overlayNode: params.nodes.addOverlay,
                value,
                getResolver: params.getResolveAddDialog,
                setResolver: (nextResolver) => {
                    params.setResolveAddDialog(nextResolver);
                },
            });
        }
        function openAddDialog() {
            return params.dialogFlowUtils.openAddDialog({
                isReady: isAddDialogReady(),
                nodes: {
                    addOverlay: params.nodes.addOverlay,
                    addUrlInput: params.nodes.addUrlInput,
                    addTitleInput: params.nodes.addTitleInput,
                    addMessageInput: params.nodes.addMessageInput,
                    addRefreshInput: params.nodes.addRefreshInput,
                },
                isHttpUrl: params.isHttpUrl,
                setStatusError: params.setStatusError,
                closeDialog: closeAddDialog,
                createResolverPromise: () => params.dialogStateUtils.createResolverPromise((nextResolver) => {
                    params.setResolveAddDialog(nextResolver);
                }),
            });
        }
        function isDeleteDialogReady() {
            return params.dialogUtils.isDeleteDialogReady({
                deleteOverlay: params.nodes.deleteOverlay,
                deleteMessage: params.nodes.deleteMessage,
                deleteCancelButton: params.nodes.deleteCancelButton,
                deleteConfirmButton: params.nodes.deleteConfirmButton,
            });
        }
        function closeDeleteDialog(value) {
            params.dialogStateUtils.closeDialogWithResolver({
                overlayNode: params.nodes.deleteOverlay,
                value,
                getResolver: params.getResolveDeleteDialog,
                setResolver: (nextResolver) => {
                    params.setResolveDeleteDialog(nextResolver);
                },
            });
        }
        function openDeleteDialog(item) {
            return params.dialogFlowUtils.openDeleteDialog({
                title: params.toCardTitle(item),
                isReady: isDeleteDialogReady(),
                nodes: {
                    deleteMessage: params.nodes.deleteMessage,
                    deleteOverlay: params.nodes.deleteOverlay,
                    deleteConfirmButton: params.nodes.deleteConfirmButton,
                },
                closeDialog: closeDeleteDialog,
                createResolverPromise: () => params.dialogStateUtils.createResolverPromise((nextResolver) => {
                    params.setResolveDeleteDialog(nextResolver);
                }),
            });
        }
        function isRenameDialogReady() {
            return params.dialogUtils.isRenameDialogReady({
                renameOverlay: params.nodes.renameOverlay,
                renameForm: params.nodes.renameForm,
                renameInput: params.nodes.renameInput,
                renameMessageInput: params.nodes.renameMessageInput,
                renameCancelButton: params.nodes.renameCancelButton,
            });
        }
        function closeRenameDialog(value) {
            params.dialogStateUtils.closeDialogWithResolver({
                overlayNode: params.nodes.renameOverlay,
                value,
                getResolver: params.getResolveRenameDialog,
                setResolver: (nextResolver) => {
                    params.setResolveRenameDialog(nextResolver);
                },
            });
        }
        function openRenameDialog(title, message) {
            return params.dialogFlowUtils.openRenameDialog({
                title,
                message,
                isReady: isRenameDialogReady(),
                nodes: {
                    renameInput: params.nodes.renameInput,
                    renameMessageInput: params.nodes.renameMessageInput,
                    renameOverlay: params.nodes.renameOverlay,
                },
                closeDialog: closeRenameDialog,
                createResolverPromise: () => params.dialogStateUtils.createResolverPromise((nextResolver) => {
                    params.setResolveRenameDialog(nextResolver);
                }),
            });
        }
        return {
            isAddDialogReady,
            closeAddDialog,
            openAddDialog,
            isDeleteDialogReady,
            closeDeleteDialog,
            openDeleteDialog,
            isRenameDialogReady,
            closeRenameDialog,
            openRenameDialog,
        };
    }
    return {
        createDialogHandlers,
    };
}
