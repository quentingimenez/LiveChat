function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
function toErrorMessage(error) {
    if (error instanceof Error && error.message) {
        return error.message;
    }
    if (typeof error === 'string') {
        return error;
    }
    return `${error}`;
}
function isTextAreaNode(value) {
    return typeof HTMLTextAreaElement === 'function' && value instanceof HTMLTextAreaElement;
}
export function createBoardLegacyPreviewMessageUtils() {
    function hasPendingPreviewMessageChanges(editor) {
        if (!editor || !isTextAreaNode(editor.inputNode)) {
            return false;
        }
        const draftValue = toTrimmedString(editor.inputNode.value);
        const savedValue = toTrimmedString(editor.lastSavedMessage);
        return draftValue !== savedValue;
    }
    async function savePreviewMessage(params) {
        const editor = params.editor;
        if (!editor || !isTextAreaNode(editor.inputNode)) {
            return false;
        }
        const itemId = toTrimmedString(editor.itemId);
        if (!itemId) {
            return false;
        }
        if (editor.saving) {
            if (params.waitForInFlight && editor.inFlightSavePromise) {
                try {
                    await editor.inFlightSavePromise;
                }
                catch {
                    // Ignore: the in-flight save already reported the failure.
                }
            }
            else {
                return !hasPendingPreviewMessageChanges(editor);
            }
        }
        const nextMessage = toTrimmedString(editor.inputNode.value);
        const previousSavedMessage = toTrimmedString(editor.lastSavedMessage);
        if (nextMessage === previousSavedMessage) {
            return true;
        }
        const nextTitle = toTrimmedString(params.resolveTitleForItem(itemId));
        editor.saving = true;
        const savePromise = (async () => {
            try {
                await params.patchItemMetadata(itemId, {
                    title: nextTitle,
                    message: nextMessage,
                }, { keepalive: params.keepalive });
                editor.lastSavedMessage = nextMessage;
                params.onSaved({
                    itemId,
                    nextTitle,
                    nextMessage,
                });
                return true;
            }
            catch (error) {
                if (!params.silent) {
                    params.setStatusError(`Enregistrement du message impossible: ${toErrorMessage(error)}`);
                }
                return false;
            }
        })();
        editor.inFlightSavePromise = savePromise;
        try {
            return await savePromise;
        }
        finally {
            if (editor.inFlightSavePromise === savePromise) {
                editor.inFlightSavePromise = null;
            }
            editor.saving = false;
            if (hasPendingPreviewMessageChanges(editor)) {
                params.scheduleAutosave(280);
            }
        }
    }
    async function flushPreviewMessageChanges(params) {
        params.clearAutosave();
        const editor = params.editor;
        if (!editor || !isTextAreaNode(editor.inputNode)) {
            return true;
        }
        // Handle race: save in progress + user typed again while request was pending.
        for (let attempt = 0; attempt < 2; attempt += 1) {
            const saved = await savePreviewMessage({
                editor,
                waitForInFlight: true,
                keepalive: params.keepalive,
                silent: params.silent,
                resolveTitleForItem: params.resolveTitleForItem,
                patchItemMetadata: params.patchItemMetadata,
                onSaved: params.onSaved,
                setStatusError: params.setStatusError,
                scheduleAutosave: params.scheduleAutosave,
            });
            if (!hasPendingPreviewMessageChanges(editor)) {
                return saved;
            }
            if (!saved) {
                return false;
            }
        }
        if (!params.silent) {
            params.setStatusError("Le message n'a pas pu être sauvegardé, action annulée.");
        }
        return false;
    }
    return {
        hasPendingPreviewMessageChanges,
        savePreviewMessage,
        flushPreviewMessageChanges,
    };
}
