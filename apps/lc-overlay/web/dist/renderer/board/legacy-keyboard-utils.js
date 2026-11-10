function toKeyString(value) {
    return typeof value === 'string' ? value : '';
}
export function createBoardLegacyKeyboardUtils() {
    function resolveDialogKeyboardAction(params) {
        const key = toKeyString(params.key);
        if (params.hasAddDialog) {
            if (key === 'Escape') {
                return {
                    shouldStopProcessing: true,
                    shouldPreventDefault: true,
                    action: 'close-add-dialog'
                };
            }
            return {
                shouldStopProcessing: true,
                shouldPreventDefault: false,
                action: 'none'
            };
        }
        if (params.hasRenameDialog) {
            if (key === 'Escape') {
                return {
                    shouldStopProcessing: true,
                    shouldPreventDefault: true,
                    action: 'close-rename-dialog'
                };
            }
            return {
                shouldStopProcessing: true,
                shouldPreventDefault: false,
                action: 'none'
            };
        }
        if (params.hasDeleteDialog) {
            if (key === 'Escape') {
                return {
                    shouldStopProcessing: true,
                    shouldPreventDefault: true,
                    action: 'close-delete-dialog-cancel'
                };
            }
            if (key === 'Enter') {
                return {
                    shouldStopProcessing: true,
                    shouldPreventDefault: true,
                    action: 'close-delete-dialog-confirm'
                };
            }
            return {
                shouldStopProcessing: true,
                shouldPreventDefault: false,
                action: 'none'
            };
        }
        return {
            shouldStopProcessing: false,
            shouldPreventDefault: false,
            action: 'none'
        };
    }
    return {
        resolveDialogKeyboardAction
    };
}
