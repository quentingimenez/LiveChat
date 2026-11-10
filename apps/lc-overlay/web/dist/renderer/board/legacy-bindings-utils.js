function toBindingsRecord(bindings) {
    if (typeof bindings !== 'object' || bindings === null) {
        return {};
    }
    const source = bindings;
    const normalized = {};
    for (const [accelerator, mappedItemId] of Object.entries(source)) {
        const normalizedAccelerator = `${accelerator || ''}`.trim();
        const normalizedItemId = `${mappedItemId || ''}`.trim();
        if (!normalizedAccelerator || !normalizedItemId) {
            continue;
        }
        normalized[normalizedAccelerator] = normalizedItemId;
    }
    return normalized;
}
function toNormalizedItemId(itemId) {
    return `${itemId || ''}`.trim();
}
export function createBoardLegacyBindingsUtils() {
    function getItemShortcuts(bindings, itemId) {
        const normalizedItemId = toNormalizedItemId(itemId);
        if (!normalizedItemId) {
            return [];
        }
        const normalizedBindings = toBindingsRecord(bindings);
        const shortcuts = [];
        for (const [accelerator, mappedItemId] of Object.entries(normalizedBindings)) {
            if (mappedItemId === normalizedItemId) {
                shortcuts.push(accelerator);
            }
        }
        shortcuts.sort((left, right) => left.localeCompare(right, undefined, { sensitivity: 'base' }));
        return shortcuts;
    }
    function removeBindingsForItem(bindings, itemId) {
        const normalizedItemId = toNormalizedItemId(itemId);
        const nextBindings = toBindingsRecord(bindings);
        if (!normalizedItemId) {
            return {
                nextBindings,
                removedCount: 0
            };
        }
        let removedCount = 0;
        for (const [accelerator, mappedItemId] of Object.entries(nextBindings)) {
            if (mappedItemId === normalizedItemId) {
                delete nextBindings[accelerator];
                removedCount += 1;
            }
        }
        return {
            nextBindings,
            removedCount
        };
    }
    function assignShortcutForItem(bindings, itemId, accelerator) {
        const normalizedItemId = toNormalizedItemId(itemId);
        const normalizedAccelerator = `${accelerator || ''}`.trim();
        const nextBindings = toBindingsRecord(bindings);
        if (!normalizedItemId || !normalizedAccelerator) {
            return {
                nextBindings
            };
        }
        for (const [existingAccelerator, mappedItemId] of Object.entries(nextBindings)) {
            if (mappedItemId === normalizedItemId || existingAccelerator === normalizedAccelerator) {
                delete nextBindings[existingAccelerator];
            }
        }
        nextBindings[normalizedAccelerator] = normalizedItemId;
        return {
            nextBindings
        };
    }
    function filterBindingsByValidItemIds(bindings, validItemIds) {
        const allowedItemIds = new Set(Array.from(validItemIds)
            .map((value) => `${value || ''}`.trim())
            .filter(Boolean));
        const nextBindings = {};
        const normalizedBindings = toBindingsRecord(bindings);
        let removedCount = 0;
        for (const [accelerator, mappedItemId] of Object.entries(normalizedBindings)) {
            if (allowedItemIds.has(mappedItemId)) {
                nextBindings[accelerator] = mappedItemId;
            }
            else {
                removedCount += 1;
            }
        }
        return {
            nextBindings,
            removedCount
        };
    }
    return {
        getItemShortcuts,
        assignShortcutForItem,
        removeBindingsForItem,
        filterBindingsByValidItemIds
    };
}
