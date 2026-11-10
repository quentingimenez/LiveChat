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
function toItemId(item) {
    if (typeof item !== 'object' || item === null) {
        return '';
    }
    const itemRecord = item;
    return toTrimmedString(itemRecord.id);
}
function toItemTitle(item) {
    if (typeof item !== 'object' || item === null) {
        return '';
    }
    const itemRecord = item;
    return toTrimmedString(itemRecord.title);
}
function toItemMessage(item) {
    if (typeof item !== 'object' || item === null) {
        return '';
    }
    const itemRecord = item;
    return toTrimmedString(itemRecord.message);
}
export function createBoardLegacyItemMetadataActionsUtils() {
    async function renameItem(params) {
        const flushed = await params.flushPreviewMessageChanges();
        if (!flushed) {
            return;
        }
        const currentTitle = toItemTitle(params.item);
        const currentMessage = toItemMessage(params.item);
        const nextMetadata = await params.openRenameDialog(currentTitle, currentMessage);
        if (nextMetadata === null) {
            return;
        }
        const itemId = toItemId(params.item);
        if (!itemId) {
            params.setStatusError('Renommage impossible: id introuvable.');
            return;
        }
        const nextTitle = toTrimmedString(nextMetadata.title);
        const nextMessage = toTrimmedString(nextMetadata.message);
        try {
            await params.patchItemMetadata(itemId, {
                title: nextTitle,
                message: nextMessage,
            });
            await params.loadItemsAndRender();
            const hasTitle = nextTitle.length > 0;
            const hasMessage = nextMessage.length > 0;
            params.setStatusSuccess(hasTitle || hasMessage
                ? `Mème mis à jour (${hasTitle ? 'nom' : 'sans nom'} / ${hasMessage ? 'message' : 'sans message'}).`
                : 'Nom et message supprimés.');
        }
        catch (error) {
            params.setStatusError(`Renommage impossible: ${toErrorMessage(error)}`);
        }
    }
    async function deleteItem(params) {
        const flushed = await params.flushPreviewMessageChanges();
        if (!flushed) {
            return;
        }
        const confirmed = await params.openDeleteDialog(params.item);
        if (!confirmed) {
            return;
        }
        const itemId = toItemId(params.item);
        if (!itemId) {
            params.setStatusError('Suppression impossible: id introuvable.');
            return;
        }
        try {
            await params.deleteItemById(itemId);
            params.clearSelectionIfMatches(itemId);
            const { nextBindings, removedCount } = params.removeBindingsForItem(itemId);
            let shortcutCleanupError = null;
            if (removedCount > 0) {
                try {
                    await params.persistBindings(nextBindings);
                }
                catch (error) {
                    shortcutCleanupError = error;
                }
            }
            await params.loadItemsAndRender();
            if (shortcutCleanupError) {
                params.setStatusError(`Mème supprimé, mais libération du raccourci impossible : ${toErrorMessage(shortcutCleanupError)}`);
                return;
            }
            params.setStatusSuccess(removedCount > 0 ? 'Mème supprimé. Raccourci libéré.' : 'Mème supprimé.');
        }
        catch (error) {
            params.setStatusError(`Suppression impossible: ${toErrorMessage(error)}`);
        }
    }
    return {
        renameItem,
        deleteItem,
    };
}
