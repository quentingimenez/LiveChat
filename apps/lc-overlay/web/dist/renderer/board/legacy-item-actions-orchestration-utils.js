export function createBoardLegacyItemActionsOrchestrationUtils() {
    function createItemActionHandlers(params) {
        async function persistBindings(nextBindings) {
            const result = await params.setMemeBindings({
                bindings: nextBindings,
            });
            if (!result || result.ok !== true) {
                const failed = Array.isArray(result?.failedAccelerators) && result.failedAccelerators.length > 0
                    ? ` (${result.failedAccelerators.join(', ')})`
                    : '';
                throw new Error(`Impossible d'appliquer les raccourcis${failed}`);
            }
            params.setBindings(result.bindings || {});
        }
        async function addItemFromLink(payload) {
            await params.itemActionUtils.addItemFromLink({
                payload,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
                createItem: (nextPayload) => params.itemUtils.createItem({
                    buildAuthedUrl: params.buildAuthedUrl,
                    payload: nextPayload,
                }),
                onItemSelected: (item) => {
                    if (!item || typeof item !== 'object') {
                        return;
                    }
                    const itemRecord = item;
                    if (!itemRecord.id) {
                        return;
                    }
                    params.setSelectedId(`${itemRecord.id}`);
                    params.setSelectedItem(item);
                },
                loadItemsAndRender: params.loadItemsAndRender,
            });
        }
        async function triggerItem(itemId) {
            await params.itemActionUtils.triggerItem({
                itemId,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                triggerMeme: async (nextItemId) => params.triggerMeme({
                    itemId: nextItemId,
                    trigger: 'ui',
                }),
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
            });
        }
        async function stopCurrentPlayback() {
            await params.itemActionUtils.stopCurrentPlayback({
                stopMemePlayback: params.stopMemePlayback,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
            });
        }
        function removeBindingsForItem(itemId) {
            return params.bindingsUtils.removeBindingsForItem(params.getBindings(), itemId);
        }
        async function syncBindingsWithBoardItems() {
            await params.bindingsActionUtils.syncBindingsWithBoardItems({
                items: params.getItems(),
                bindings: params.getBindings(),
                filterBindingsByValidItemIds: params.bindingsUtils.filterBindingsByValidItemIds,
                persistBindings,
            });
        }
        async function clearShortcutForItem(itemId) {
            await params.bindingsActionUtils.clearShortcutForItem({
                itemId,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                removeBindingsForItem,
                persistBindings,
                renderList: params.renderList,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
            });
        }
        async function renameItem(item) {
            await params.itemMetadataActionsUtils.renameItem({
                item,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                openRenameDialog: params.openRenameDialog,
                patchItemMetadata: params.patchItemMetadata,
                loadItemsAndRender: params.loadItemsAndRender,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
            });
        }
        async function deleteItem(item) {
            await params.itemMetadataActionsUtils.deleteItem({
                item,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                openDeleteDialog: params.openDeleteDialog,
                deleteItemById: (itemId) => params.itemUtils.deleteItem({
                    buildAuthedUrl: params.buildAuthedUrl,
                    itemId,
                }),
                clearSelectionIfMatches: (itemId) => {
                    if (`${params.getSelectedId() || ''}` === `${itemId || ''}`) {
                        params.setSelectedId(null);
                        params.setSelectedItem(null);
                        params.setPreviewMediaKey(null);
                    }
                },
                removeBindingsForItem,
                persistBindings,
                loadItemsAndRender: params.loadItemsAndRender,
                setStatusSuccess: params.setStatusSuccess,
                setStatusError: params.setStatusError,
            });
        }
        return {
            addItemFromLink,
            persistBindings,
            triggerItem,
            stopCurrentPlayback,
            removeBindingsForItem,
            syncBindingsWithBoardItems,
            clearShortcutForItem,
            renameItem,
            deleteItem,
        };
    }
    return {
        createItemActionHandlers,
    };
}
