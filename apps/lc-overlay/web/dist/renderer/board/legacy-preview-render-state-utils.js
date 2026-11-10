function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
export function createBoardLegacyPreviewRenderStateUtils() {
    function renderEmptyPreviewState(params) {
        if (params.previewStageNode instanceof HTMLElement) {
            params.previewStageNode.innerHTML = '';
            params.previewStageNode.classList.remove('preview-stage-visual');
            params.previewStageNode.appendChild(params.createEmptyStateNode({
                text: params.emptyText,
            }));
        }
        if (params.selectedTitleNode instanceof HTMLElement) {
            params.selectedTitleNode.textContent = '';
        }
        if (params.selectedMetaNode instanceof HTMLElement) {
            params.selectedMetaNode.textContent = params.emptyMetaText;
        }
    }
    function shouldResetPreviewMessageEditor(params) {
        const editorRecord = toRecord(params.previewMessageEditor);
        const editorItemId = toTrimmedString(editorRecord.itemId);
        const selectedItemId = toTrimmedString(params.selectedItemId);
        if (!editorItemId) {
            return false;
        }
        return editorItemId !== selectedItemId;
    }
    function buildPreviewMediaContext(params) {
        const selectedRecord = toRecord(params.selectedItem);
        const itemId = toTrimmedString(selectedRecord.id);
        const mediaAssetId = toTrimmedString(selectedRecord.mediaAssetId);
        const mediaRecord = toRecord(selectedRecord.media);
        const mediaKind = params.normalizeMediaKind(mediaRecord.kind);
        const mediaUrl = params.buildAuthedUrl(`/overlay/meme-board/media/${mediaAssetId}`).toString();
        const mediaKey = `${itemId}|${mediaAssetId}|${mediaKind}|${mediaUrl}`;
        return {
            mediaKind,
            mediaUrl,
            mediaKey,
        };
    }
    return {
        renderEmptyPreviewState,
        shouldResetPreviewMessageEditor,
        buildPreviewMediaContext,
    };
}
