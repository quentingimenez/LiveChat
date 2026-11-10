function toSelectedItem(item) {
    if (typeof item !== 'object' || item === null) {
        return null;
    }
    const record = item;
    const itemId = `${record.id || ''}`.trim();
    if (!itemId) {
        return null;
    }
    return { id: itemId };
}
export function createBoardLegacyPreviewActionUtils() {
    function createPreviewControlHandlers(params) {
        return {
            onPlay: () => {
                void (async () => {
                    const selectedItem = params.findSelectedItem();
                    const selected = toSelectedItem(selectedItem);
                    if (!selected) {
                        return;
                    }
                    await params.triggerItem(selected.id);
                })();
            },
            onRename: () => {
                void (async () => {
                    const selectedItem = params.findSelectedItem();
                    if (!selectedItem) {
                        return;
                    }
                    await params.renameItem(selectedItem);
                })();
            },
            onBindShortcut: () => {
                void (async () => {
                    const selectedItem = params.findSelectedItem();
                    const selected = toSelectedItem(selectedItem);
                    if (!selected) {
                        return;
                    }
                    const flushed = await params.flushPreviewMessageChanges();
                    if (!flushed) {
                        return;
                    }
                    params.beginCaptureForItem(selected.id);
                })();
            },
            onClearShortcut: () => {
                void (async () => {
                    const selectedItem = params.findSelectedItem();
                    const selected = toSelectedItem(selectedItem);
                    if (!selected) {
                        return;
                    }
                    await params.clearShortcutForItem(selected.id);
                })();
            },
            onDelete: () => {
                void (async () => {
                    const selectedItem = params.findSelectedItem();
                    if (!selectedItem) {
                        return;
                    }
                    await params.deleteItem(selectedItem);
                })();
            },
        };
    }
    return {
        createPreviewControlHandlers,
    };
}
