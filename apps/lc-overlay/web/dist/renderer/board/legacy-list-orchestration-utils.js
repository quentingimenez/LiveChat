export function createBoardLegacyListOrchestrationUtils() {
    function createRenderList(params) {
        function renderList() {
            const listActionHandlers = params.listActionUtils.createListActionHandlers({
                getSelectedId: params.getSelectedId,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                clearStatus: params.clearStatus,
                selectItem: params.selectItem,
                renderList,
                triggerItem: params.triggerItem,
                renameItem: params.renameItem,
                beginCaptureForItem: params.beginCaptureForItem,
                deleteItem: params.deleteItem,
            });
            params.listRenderUtils.renderItemList({
                itemsListNode: params.itemsListNode,
                countPillNode: params.countPillNode,
                items: params.getItems(),
                total: params.getTotal(),
                selectedId: params.getSelectedId(),
                getItemShortcuts: params.getItemShortcuts,
                toCardTitle: params.toCardTitle,
                toSafeDateLabel: params.toSafeDateLabel,
                toMessagePreview: params.toMessagePreview,
                createEmptyStateNode: params.createEmptyStateNode,
                createItemCard: params.createItemCard,
                onSelectItem: listActionHandlers.onSelectItem,
                onTriggerItem: listActionHandlers.onTriggerItem,
                onRenameItem: listActionHandlers.onRenameItem,
                onBindItem: listActionHandlers.onBindItem,
                onDeleteItem: listActionHandlers.onDeleteItem,
                onRenderPreview: params.renderPreview,
            });
        }
        return renderList;
    }
    return {
        createRenderList,
    };
}
