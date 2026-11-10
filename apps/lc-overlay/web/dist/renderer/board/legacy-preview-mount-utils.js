export function createBoardLegacyPreviewMountUtils() {
    function mountPreviewMediaIfNeeded(params) {
        if (params.currentPreviewMediaKey === params.nextPreviewMediaKey) {
            return;
        }
        params.setPreviewMediaKey(params.nextPreviewMediaKey);
        if (!(params.previewStageNode instanceof HTMLElement)) {
            return;
        }
        params.previewStageNode.innerHTML = '';
        params.previewStageNode.classList.toggle('preview-stage-visual', params.mediaKind !== 'audio');
        const mediaNode = params.createPreviewMediaNode({
            mediaKind: params.mediaKind,
            mediaUrl: params.mediaUrl,
            title: params.title,
            onMediaReady: params.schedulePreviewLayoutRefresh,
        });
        const messageEditor = params.createPreviewMessageEditor({
            initialMessage: params.initialMessage,
            onInput: (inputNode) => {
                const editor = params.getPreviewMessageEditor();
                if (!editor || editor.inputNode !== inputNode) {
                    return;
                }
                params.schedulePreviewMessageAutosave();
            },
            onBlur: (inputNode) => {
                const editor = params.getPreviewMessageEditor();
                if (!editor || editor.inputNode !== inputNode) {
                    return;
                }
                params.schedulePreviewMessageAutosave(220);
            },
        });
        params.setPreviewMessageEditor({
            itemId: `${params.selectedItemId || ''}`.trim(),
            inputNode: messageEditor.inputNode,
            lastSavedMessage: `${messageEditor.initialMessage || ''}`.trim(),
            saving: false,
            inFlightSavePromise: null,
        });
        const controlsNode = params.createPreviewControls(params.createPreviewControlHandlers());
        params.previewStageNode.appendChild(mediaNode);
        params.previewStageNode.appendChild(messageEditor.rootNode);
        params.previewStageNode.appendChild(controlsNode);
    }
    return {
        mountPreviewMediaIfNeeded,
    };
}
