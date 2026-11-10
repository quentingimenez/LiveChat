function toErrorMessage(error) {
    if (error instanceof Error && error.message) {
        return error.message;
    }
    if (typeof error === 'string') {
        return error;
    }
    return `${error}`;
}
function collectValidItemIds(items) {
    const values = Array.isArray(items) ? items : [];
    const validItemIds = new Set();
    for (const item of values) {
        const itemId = `${item?.id || ''}`.trim();
        if (itemId) {
            validItemIds.add(itemId);
        }
    }
    return validItemIds;
}
export function createBoardLegacyBindingsActionUtils() {
    async function syncBindingsWithBoardItems(params) {
        const validItemIds = collectValidItemIds(params.items);
        const { nextBindings, removedCount } = params.filterBindingsByValidItemIds(params.bindings, validItemIds);
        if (removedCount === 0) {
            return;
        }
        await params.persistBindings(nextBindings);
    }
    async function clearShortcutForItem(params) {
        const flushed = await params.flushPreviewMessageChanges();
        if (!flushed) {
            return;
        }
        const { nextBindings, removedCount } = params.removeBindingsForItem(params.itemId);
        if (removedCount === 0) {
            params.setStatusSuccess('Aucun raccourci à retirer pour ce mème.');
            return;
        }
        try {
            await params.persistBindings(nextBindings);
            params.renderList();
            params.setStatusSuccess('Raccourci retiré.');
        }
        catch (error) {
            params.setStatusError(toErrorMessage(error) || 'Impossible de retirer le raccourci.');
        }
    }
    return {
        syncBindingsWithBoardItems,
        clearShortcutForItem,
    };
}
