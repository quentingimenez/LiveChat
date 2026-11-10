export function createBoardLegacyLoadSearchUtils() {
    async function loadItemsAndRender(params) {
        const requestId = params.state.itemsLoadRequestId + 1;
        params.state.itemsLoadRequestId = requestId;
        try {
            const fetched = await params.fetchItems(params.state.search);
            if (requestId !== params.state.itemsLoadRequestId) {
                return;
            }
            params.state.items = fetched.items;
            params.state.total = fetched.total;
            const nextSelection = params.resolveSelectionAfterItemsLoad({
                items: params.state.items,
                selectedId: params.state.selectedId,
                selectedItem: params.state.selectedItem,
            });
            params.state.selectedId = nextSelection.selectedId;
            params.state.selectedItem = nextSelection.selectedItem;
            await params.syncBindingsWithBoardItems();
            if (requestId !== params.state.itemsLoadRequestId) {
                return;
            }
            params.renderList();
            params.clearStatus();
        }
        catch (error) {
            const errorMessage = error instanceof Error ? error.message : '';
            params.setStatusError(errorMessage || 'Erreur de chargement de la mème board.');
        }
    }
    async function runSearchNow(params) {
        params.clearSearchDebounce();
        const flushed = await params.flushPreviewMessageChanges();
        if (!flushed) {
            return;
        }
        params.state.search = params.normalizeSearchQuery(params.searchInputValue);
        await params.loadItemsAndRender();
    }
    function scheduleInstantSearch(params) {
        params.clearSearchDebounce();
        params.state.searchDebounceTimeoutId = params.scheduleDebouncedTask({
            delayMs: params.delayMs,
            setTimer: params.setTimer,
            runTask: () => {
                params.state.searchDebounceTimeoutId = null;
                void (async () => {
                    const flushed = await params.flushPreviewMessageChanges();
                    if (!flushed) {
                        return;
                    }
                    params.state.search = params.normalizeSearchQuery(params.getSearchInputValue());
                    await params.loadItemsAndRender();
                })();
            },
        });
    }
    return {
        loadItemsAndRender,
        runSearchNow,
        scheduleInstantSearch,
    };
}
