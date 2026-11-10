function toItemId(value) {
    if (typeof value !== 'object' || value === null) {
        return '';
    }
    const record = value;
    return `${record.id || ''}`.trim();
}
export function createBoardLegacyListActionUtils() {
    function createListActionHandlers(params) {
        return {
            onSelectItem: (item) => {
                void (async () => {
                    const nextItemId = toItemId(item);
                    if (`${params.getSelectedId() || ''}` !== nextItemId) {
                        const flushed = await params.flushPreviewMessageChanges();
                        if (!flushed) {
                            return;
                        }
                        params.clearStatus();
                    }
                    params.selectItem(item);
                    params.renderList();
                })();
            },
            onTriggerItem: (itemId) => {
                void params.triggerItem(itemId);
            },
            onRenameItem: (item) => {
                void params.renameItem(item);
            },
            onBindItem: (itemId) => {
                void (async () => {
                    const flushed = await params.flushPreviewMessageChanges();
                    if (!flushed) {
                        return;
                    }
                    params.beginCaptureForItem(itemId);
                })();
            },
            onDeleteItem: (item) => {
                void params.deleteItem(item);
            },
        };
    }
    return {
        createListActionHandlers,
    };
}
