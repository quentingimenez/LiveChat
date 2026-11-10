function toItemArray(items) {
    return Array.isArray(items) ? items : [];
}
function toItemId(item) {
    if (typeof item !== 'object' || item === null) {
        return '';
    }
    const record = item;
    return `${record.id || ''}`.trim();
}
export function createBoardLegacyListRenderUtils() {
    function renderItemList(params) {
        const itemsListNode = params.itemsListNode;
        const countPillNode = params.countPillNode;
        if (itemsListNode instanceof HTMLElement) {
            itemsListNode.innerHTML = '';
        }
        if (countPillNode instanceof HTMLElement) {
            countPillNode.textContent = `${params.total}`;
        }
        const items = toItemArray(params.items);
        if (items.length === 0) {
            if (itemsListNode instanceof HTMLElement) {
                const emptyNode = params.createEmptyStateNode({
                    text: 'Aucun mème dans la board pour cette recherche.',
                });
                itemsListNode.appendChild(emptyNode);
            }
            params.onRenderPreview();
            return;
        }
        const normalizedSelectedId = `${params.selectedId || ''}`;
        for (const item of items) {
            const itemId = toItemId(item);
            const shortcuts = params.getItemShortcuts(itemId);
            const card = params.createItemCard({
                item,
                isSelected: itemId === normalizedSelectedId,
                shortcuts,
                toCardTitle: params.toCardTitle,
                toSafeDateLabel: params.toSafeDateLabel,
                toMessagePreview: params.toMessagePreview,
                onSelect: () => {
                    params.onSelectItem(item);
                },
                onTrigger: () => {
                    params.onTriggerItem(itemId);
                },
                onRename: () => {
                    params.onRenameItem(item);
                },
                onBind: () => {
                    params.onBindItem(itemId);
                },
                onDelete: () => {
                    params.onDeleteItem(item);
                },
            });
            if (itemsListNode instanceof HTMLElement) {
                itemsListNode.appendChild(card);
            }
        }
        params.onRenderPreview();
    }
    return {
        renderItemList,
    };
}
