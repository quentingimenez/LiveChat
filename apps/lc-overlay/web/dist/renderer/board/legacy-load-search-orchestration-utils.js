export function createBoardLegacyLoadSearchOrchestrationUtils() {
    function createLoadSearchHandlers(params) {
        async function fetchItems(searchQuery) {
            return params.itemUtils.fetchItems({
                buildAuthedUrl: params.buildAuthedUrl,
                searchQuery,
            });
        }
        async function patchItemMetadata(itemId, payload, options = {}) {
            await params.itemUtils.patchItemMetadata({
                buildAuthedUrl: params.buildAuthedUrl,
                itemId,
                payload,
                options,
            });
        }
        function applyLocalItemMetadata(itemId, nextTitle, nextMessage) {
            const result = params.itemUtils.applyLocalItemMetadata({
                items: params.state.items,
                selectedItem: params.state.selectedItem,
                itemId,
                nextTitle,
                nextMessage,
            });
            params.setItems(result.items);
            params.setSelectedItem(result.selectedItem);
            return result.updated;
        }
        async function loadItemsAndRender() {
            await params.loadSearchUtils.loadItemsAndRender({
                state: params.state,
                fetchItems,
                resolveSelectionAfterItemsLoad: params.resolveSelectionAfterItemsLoad,
                syncBindingsWithBoardItems: params.syncBindingsWithBoardItems,
                renderList: params.renderList,
                clearStatus: params.clearStatus,
                setStatusError: params.setStatusError,
            });
        }
        async function runSearchNow() {
            await params.loadSearchUtils.runSearchNow({
                state: params.state,
                searchInputValue: params.getSearchInputValue(),
                clearSearchDebounce: params.clearSearchDebounce,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                normalizeSearchQuery: params.searchUtils.normalizeSearchQuery,
                loadItemsAndRender,
            });
        }
        function scheduleInstantSearch() {
            params.loadSearchUtils.scheduleInstantSearch({
                state: params.state,
                getSearchInputValue: params.getSearchInputValue,
                clearSearchDebounce: params.clearSearchDebounce,
                flushPreviewMessageChanges: params.flushPreviewMessageChanges,
                normalizeSearchQuery: params.searchUtils.normalizeSearchQuery,
                loadItemsAndRender,
                scheduleDebouncedTask: params.scheduleDebouncedTask,
                delayMs: params.instantSearchDebounceMs,
                setTimer: params.setTimeout,
            });
        }
        return {
            fetchItems,
            patchItemMetadata,
            applyLocalItemMetadata,
            loadItemsAndRender,
            runSearchNow,
            scheduleInstantSearch,
        };
    }
    return {
        createLoadSearchHandlers,
    };
}
