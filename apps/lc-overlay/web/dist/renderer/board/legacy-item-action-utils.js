function toErrorMessage(error) {
    if (error instanceof Error && error.message) {
        return error.message;
    }
    if (typeof error === 'string') {
        return error;
    }
    return `${error}`;
}
export function createBoardLegacyItemActionUtils() {
    async function addItemFromLink(params) {
        try {
            const flushed = await params.flushPreviewMessageChanges();
            if (!flushed) {
                return;
            }
            params.setStatusSuccess('Ajout du mème en cours...');
            const body = await params.createItem({
                url: params.payload.url,
                title: params.payload.title,
                message: params.payload.message,
                forceRefresh: params.payload.forceRefresh,
            });
            if (body?.item?.id) {
                params.onItemSelected(body.item);
            }
            await params.loadItemsAndRender();
            params.setStatusSuccess(body?.created === false ? 'Mème déjà présent dans la board.' : 'Mème ajouté dans la board.');
        }
        catch (error) {
            params.setStatusError(`Ajout impossible: ${toErrorMessage(error)}`);
        }
    }
    async function triggerItem(params) {
        try {
            const flushed = await params.flushPreviewMessageChanges();
            if (!flushed) {
                return;
            }
            const result = await params.triggerMeme(params.itemId);
            if (!result?.ok) {
                throw new Error(result?.reason || 'socket_offline');
            }
            params.setStatusSuccess('Mème envoyé dans la file.');
        }
        catch (error) {
            params.setStatusError(`Échec trigger : ${toErrorMessage(error)}`);
        }
    }
    async function stopCurrentPlayback(params) {
        try {
            const result = await params.stopMemePlayback();
            if (!result?.ok) {
                throw new Error(result?.reason || 'socket_offline');
            }
            params.setStatusSuccess('Lecture en cours stoppée.');
        }
        catch (error) {
            params.setStatusError(`Stop impossible: ${toErrorMessage(error)}`);
        }
    }
    return {
        addItemFromLink,
        triggerItem,
        stopCurrentPlayback,
    };
}
