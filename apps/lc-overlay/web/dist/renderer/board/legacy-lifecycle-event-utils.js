function toDialogAction(value) {
    return typeof value === 'string' ? value : '';
}
function toBoolean(value) {
    return value === true;
}
function normalizeDialogKeyboardResolution(value) {
    const record = typeof value === 'object' && value !== null ? value : {};
    return {
        shouldStopProcessing: toBoolean(record.shouldStopProcessing),
        shouldPreventDefault: toBoolean(record.shouldPreventDefault),
        action: toDialogAction(record.action),
    };
}
function normalizeCaptureKeyResolution(value) {
    const record = typeof value === 'object' && value !== null ? value : {};
    return {
        kind: toDialogAction(record.kind),
        nextPendingAccelerator: typeof record.nextPendingAccelerator === 'string' ? record.nextPendingAccelerator : null,
        nextPendingDisplay: typeof record.nextPendingDisplay === 'string' ? record.nextPendingDisplay : '',
        statusMessage: typeof record.statusMessage === 'string' ? record.statusMessage : '',
    };
}
export function createBoardLegacyLifecycleEventUtils() {
    function bindKeyboardHandler(params) {
        window.addEventListener('keydown', (event) => {
            const dialogKeyboardResolution = normalizeDialogKeyboardResolution(params.resolveDialogKeyboardAction(event.key));
            if (dialogKeyboardResolution.shouldStopProcessing) {
                if (dialogKeyboardResolution.shouldPreventDefault) {
                    event.preventDefault();
                }
                if (dialogKeyboardResolution.action === 'close-add-dialog') {
                    params.closeAddDialog();
                }
                else if (dialogKeyboardResolution.action === 'close-rename-dialog') {
                    params.closeRenameDialog();
                }
                else if (dialogKeyboardResolution.action === 'close-delete-dialog-cancel') {
                    params.closeDeleteDialogCancel();
                }
                else if (dialogKeyboardResolution.action === 'close-delete-dialog-confirm') {
                    params.closeDeleteDialogConfirm();
                }
                return;
            }
            if (!params.isCaptureActive()) {
                return;
            }
            event.preventDefault();
            event.stopPropagation();
            const resolution = normalizeCaptureKeyResolution(params.resolveCaptureKeyInput(event));
            if (resolution.kind === 'cancelled') {
                params.onCaptureCancelled();
                return;
            }
            params.onCaptureUpdated({
                nextPendingAccelerator: resolution.nextPendingAccelerator,
                nextPendingDisplay: resolution.nextPendingDisplay,
                statusMessage: resolution.statusMessage,
            });
        });
    }
    function bindLifecycleHandlers(params) {
        document.addEventListener('visibilitychange', () => {
            if (document.visibilityState === 'hidden') {
                params.flushPreviewMessageOnLifecycleExit();
            }
        });
        window.addEventListener('pagehide', () => {
            params.flushPreviewMessageOnLifecycleExit();
        });
        window.addEventListener('beforeunload', () => {
            params.flushPreviewMessageOnLifecycleExit();
        });
        window.addEventListener('resize', () => {
            params.schedulePreviewLayoutRefresh();
        });
    }
    return {
        bindKeyboardHandler,
        bindLifecycleHandlers,
    };
}
