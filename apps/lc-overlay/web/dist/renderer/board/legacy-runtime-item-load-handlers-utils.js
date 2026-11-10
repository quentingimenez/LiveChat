function requireUtility(utility, utilityName) {
    if (utility === undefined || utility === null) {
        throw new Error(`Board legacy utility unavailable: ${utilityName}`);
    }
    return utility;
}
export function createBoardLegacyRuntimeItemLoadHandlersUtils() {
    function createRuntimeItemLoadHandlers(params) {
        const itemUtils = requireUtility(window.__boardLegacyItemUtils, 'item');
        const loadSearchUtils = requireUtility(window.__boardLegacyLoadSearchUtils, 'load-search');
        const searchUtils = requireUtility(window.__boardLegacySearchUtils, 'search');
        const selectionUtils = requireUtility(window.__boardLegacySelectionUtils, 'selection');
        const loadSearchOrchestrationUtils = requireUtility(window.__boardLegacyLoadSearchOrchestrationUtils, 'load-search-orchestration');
        const itemActionUtils = requireUtility(window.__boardLegacyItemActionUtils, 'item-action');
        const itemMetadataActionsUtils = requireUtility(window.__boardLegacyItemMetadataActionsUtils, 'item-metadata-action');
        const bindingsUtils = requireUtility(window.__boardLegacyBindingsUtils, 'bindings');
        const bindingsActionUtils = requireUtility(window.__boardLegacyBindingsActionUtils, 'bindings-action');
        const itemActionsOrchestrationUtils = requireUtility(window.__boardLegacyItemActionsOrchestrationUtils, 'item-actions-orchestration');
        let syncBindingsWithBoardItems = async () => { };
        const loadSearchHandlers = loadSearchOrchestrationUtils.createLoadSearchHandlers({
            itemUtils,
            loadSearchUtils,
            searchUtils,
            state: params.state,
            buildAuthedUrl: params.buildAuthedUrl,
            getSearchInputValue: () => `${params.nodes.searchInput?.value || ''}`,
            clearSearchDebounce: params.clearSearchDebounce,
            flushPreviewMessageChanges: params.flushPreviewMessageChanges,
            resolveSelectionAfterItemsLoad: selectionUtils.resolveSelectionAfterItemsLoad,
            syncBindingsWithBoardItems: async () => {
                await syncBindingsWithBoardItems();
            },
            renderList: params.renderList,
            clearStatus: params.clearStatus,
            setStatusError: params.setStatusError,
            scheduleDebouncedTask: searchUtils.scheduleDebouncedTask,
            instantSearchDebounceMs: params.instantSearchDebounceMs,
            setTimeout: params.setTimeout,
            setItems: (nextItems) => {
                params.state.items = nextItems;
            },
            setSelectedItem: (nextSelectedItem) => {
                params.state.selectedItem = nextSelectedItem;
            },
        });
        const itemActionHandlers = itemActionsOrchestrationUtils.createItemActionHandlers({
            itemUtils,
            itemActionUtils,
            itemMetadataActionsUtils,
            bindingsUtils,
            bindingsActionUtils,
            buildAuthedUrl: params.buildAuthedUrl,
            getBindings: () => params.state.bindings,
            setBindings: (nextBindings) => {
                params.state.bindings = nextBindings;
            },
            getItems: () => params.state.items,
            getSelectedId: () => params.state.selectedId,
            setSelectedId: (nextSelectedId) => {
                params.state.selectedId = nextSelectedId;
            },
            setSelectedItem: (nextSelectedItem) => {
                params.state.selectedItem = nextSelectedItem;
            },
            setPreviewMediaKey: (nextPreviewMediaKey) => {
                params.state.previewMediaKey = nextPreviewMediaKey;
            },
            flushPreviewMessageChanges: params.flushPreviewMessageChanges,
            loadItemsAndRender: loadSearchHandlers.loadItemsAndRender,
            openRenameDialog: params.openRenameDialog,
            openDeleteDialog: params.openDeleteDialog,
            patchItemMetadata: loadSearchHandlers.patchItemMetadata,
            renderList: params.renderList,
            setStatusSuccess: params.setStatusSuccess,
            setStatusError: params.setStatusError,
            setMemeBindings: ((payload) => params.setMemeBindings(payload)),
            triggerMeme: ((payload) => params.triggerMeme(payload)),
            stopMemePlayback: (() => params.stopMemePlayback()),
        });
        syncBindingsWithBoardItems = itemActionHandlers.syncBindingsWithBoardItems;
        return {
            patchItemMetadata: loadSearchHandlers.patchItemMetadata,
            applyLocalItemMetadata: loadSearchHandlers.applyLocalItemMetadata,
            loadItemsAndRender: loadSearchHandlers.loadItemsAndRender,
            runSearchNow: loadSearchHandlers.runSearchNow,
            scheduleInstantSearch: loadSearchHandlers.scheduleInstantSearch,
            addItemFromLink: itemActionHandlers.addItemFromLink,
            persistBindings: itemActionHandlers.persistBindings,
            triggerItem: itemActionHandlers.triggerItem,
            stopCurrentPlayback: itemActionHandlers.stopCurrentPlayback,
            syncBindingsWithBoardItems: itemActionHandlers.syncBindingsWithBoardItems,
            clearShortcutForItem: itemActionHandlers.clearShortcutForItem,
            renameItem: itemActionHandlers.renameItem,
            deleteItem: itemActionHandlers.deleteItem,
        };
    }
    return {
        createRuntimeItemLoadHandlers,
    };
}
