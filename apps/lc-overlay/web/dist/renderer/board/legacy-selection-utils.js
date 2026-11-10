function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function toItemArray(items) {
    return Array.isArray(items) ? items : [];
}
function toItemId(item) {
    const itemRecord = toRecord(item);
    return `${itemRecord.id || ''}`.trim();
}
function toSelectedId(selectedId) {
    if (typeof selectedId !== 'string') {
        return null;
    }
    const normalized = selectedId.trim();
    return normalized ? normalized : null;
}
function findItemById(items, targetId) {
    if (!targetId) {
        return null;
    }
    return items.find((item) => toItemId(item) === targetId) || null;
}
export function createBoardLegacySelectionUtils() {
    function resolveSelectedItemForPreview(params) {
        const items = toItemArray(params.items);
        const selectedId = toSelectedId(params.selectedId);
        const selectedFromList = findItemById(items, selectedId);
        if (selectedFromList) {
            return {
                selectedItem: selectedFromList,
                syncedSelectedItem: selectedFromList
            };
        }
        const currentSelectedItem = params.selectedItem;
        const currentSelectedItemId = toItemId(currentSelectedItem);
        if (currentSelectedItemId && (!selectedId || currentSelectedItemId === selectedId)) {
            return {
                selectedItem: currentSelectedItem,
                syncedSelectedItem: currentSelectedItem
            };
        }
        return {
            selectedItem: null,
            syncedSelectedItem: params.selectedItem
        };
    }
    function resolveSelectionAfterItemsLoad(params) {
        const items = toItemArray(params.items);
        const selectedId = toSelectedId(params.selectedId);
        const selectedItem = params.selectedItem;
        const selectedInList = findItemById(items, selectedId);
        if (selectedInList) {
            return {
                selectedId,
                selectedItem: selectedInList
            };
        }
        if (!selectedId && !selectedItem && items.length > 0) {
            const firstItem = items[0];
            const firstItemId = toItemId(firstItem);
            return {
                selectedId: firstItemId || null,
                selectedItem: firstItem
            };
        }
        return {
            selectedId,
            selectedItem
        };
    }
    return {
        resolveSelectedItemForPreview,
        resolveSelectionAfterItemsLoad
    };
}
