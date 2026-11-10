function toItemRecord(value) {
    if (!value || typeof value !== 'object') {
        return {};
    }
    return value;
}
function toItemId(value) {
    return `${value || ''}`;
}
export function createBoardLegacyPreviewRenderOrchestrationUtils() {
    function renderPreview(params) {
        const selectedItem = params.findSelectedItem();
        if (!selectedItem) {
            if (params.hasPendingPreviewMessageChanges()) {
                const detachedEditor = params.getPreviewMessageEditor();
                params.clearPreviewMessageAutosave();
                void params.savePreviewMessage({
                    editor: detachedEditor,
                    waitForInFlight: false,
                    keepalive: true,
                    silent: true,
                });
            }
            params.resetPreviewMessageEditor();
            params.setPreviewMediaKey(null);
            params.renderEmptyPreviewState({
                previewStageNode: params.previewStageNode,
                selectedTitleNode: params.selectedTitleNode,
                selectedMetaNode: params.selectedMetaNode,
                createEmptyStateNode: params.createEmptyStateNode,
                emptyText: 'Sélectionne un mème pour voir son média et gérer son raccourci.',
                emptyMetaText: 'Aucun élément sélectionné.',
            });
            params.refreshPreviewLayoutObserverTargets();
            return;
        }
        const selectedItemRecord = toItemRecord(selectedItem);
        if (params.shouldResetPreviewMessageEditor({
            previewMessageEditor: params.getPreviewMessageEditor(),
            selectedItemId: selectedItemRecord.id,
        })) {
            const detachedEditor = params.getPreviewMessageEditor();
            if (params.hasPendingPreviewMessageChanges(detachedEditor)) {
                params.clearPreviewMessageAutosave();
                void params.savePreviewMessage({
                    editor: detachedEditor,
                    waitForInFlight: false,
                    keepalive: true,
                    silent: true,
                });
            }
            params.resetPreviewMessageEditor();
        }
        const previewMedia = params.buildPreviewMediaContext({
            selectedItem,
            normalizeMediaKind: params.normalizeMediaKind,
            buildAuthedUrl: params.buildAuthedUrl,
        });
        params.mountPreviewMediaIfNeeded({
            previewStageNode: params.previewStageNode,
            currentPreviewMediaKey: params.getPreviewMediaKey(),
            nextPreviewMediaKey: previewMedia.mediaKey,
            mediaKind: previewMedia.mediaKind,
            mediaUrl: previewMedia.mediaUrl,
            title: params.toCardTitle(selectedItem),
            selectedItemId: toItemId(selectedItemRecord.id),
            initialMessage: `${selectedItemRecord.message || ''}`.trim(),
            getPreviewMessageEditor: params.getPreviewMessageEditor,
            setPreviewMessageEditor: (nextEditor) => {
                params.setPreviewMessageEditor(nextEditor);
            },
            setPreviewMediaKey: (nextKey) => {
                params.setPreviewMediaKey(nextKey);
            },
            schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
            schedulePreviewMessageAutosave: params.schedulePreviewMessageAutosave,
            createPreviewMediaNode: params.createPreviewMediaNode,
            createPreviewMessageEditor: params.createPreviewMessageEditor,
            createPreviewControls: params.createPreviewControls,
            createPreviewControlHandlers: params.createPreviewControlHandlers,
        });
        params.applyPreviewVolume();
        params.refreshPreviewLayoutObserverTargets();
        params.schedulePreviewLayoutRefresh();
        const shortcuts = params.getItemShortcuts(selectedItemRecord.id);
        const hasMessage = params.toMessagePreview(selectedItemRecord.message).length > 0;
        if (params.selectedTitleNode instanceof HTMLElement) {
            params.selectedTitleNode.textContent = `${params.toCardTitle(selectedItem)}`;
        }
        params.selectedMetaNode.textContent = params.buildPreviewMetaText({
            mediaKind: selectedItemRecord.media?.kind,
            hasMessage,
            shortcuts,
        });
    }
    return {
        renderPreview,
    };
}
