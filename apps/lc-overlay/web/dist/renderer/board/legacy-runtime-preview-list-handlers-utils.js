function requireUtility(utility, utilityName) {
    if (utility === undefined || utility === null) {
        throw new Error(`Board legacy utility unavailable: ${utilityName}`);
    }
    return utility;
}
export function createBoardLegacyRuntimePreviewListHandlersUtils() {
    function createRuntimePreviewListHandlers(params) {
        const previewMessageUtils = requireUtility(window.__boardLegacyPreviewMessageUtils, 'preview-message');
        const previewMessageOrchestrationUtils = requireUtility(window.__boardLegacyPreviewMessageOrchestrationUtils, 'preview-message-orchestration');
        const previewRenderOrchestrationUtils = requireUtility(window.__boardLegacyPreviewRenderOrchestrationUtils, 'preview-render-orchestration');
        const previewRenderStateUtils = requireUtility(window.__boardLegacyPreviewRenderStateUtils, 'preview-render-state');
        const previewMountUtils = requireUtility(window.__boardLegacyPreviewMountUtils, 'preview-mount');
        const previewControlsUtils = requireUtility(window.__boardLegacyPreviewControlsUtils, 'preview-controls');
        const previewActionUtils = requireUtility(window.__boardLegacyPreviewActionUtils, 'preview-action');
        const renderUtils = requireUtility(window.__boardLegacyRenderUtils, 'render');
        const listActionUtils = requireUtility(window.__boardLegacyListActionUtils, 'list-action');
        const listRenderUtils = requireUtility(window.__boardLegacyListRenderUtils, 'list-render');
        const listOrchestrationUtils = requireUtility(window.__boardLegacyListOrchestrationUtils, 'list-orchestration');
        let renderList = () => { };
        const previewMessageHandlers = previewMessageOrchestrationUtils.createPreviewMessageHandlers({
            previewMessageUtils,
            defaultAutosaveDelayMs: params.defaultAutosaveDelayMs,
            getPreviewMessageEditor: () => params.state.previewMessageEditor,
            getPreviewMessageAutosaveTimeoutId: () => params.state.previewMessageAutosaveTimeoutId,
            setPreviewMessageAutosaveTimeoutId: (nextTimeoutId) => {
                params.state.previewMessageAutosaveTimeoutId = nextTimeoutId;
            },
            setTimeout: params.setTimeout,
            clearTimeout: params.clearTimeout,
            resolvePreviewMessageItemTitle: params.resolvePreviewMessageItemTitle,
            patchItemMetadata: (itemId, payload, options) => params.getPatchItemMetadata()(itemId, payload, options),
            applyLocalItemMetadata: (itemId, nextTitle, nextMessage) => params.getApplyLocalItemMetadata()(itemId, nextTitle, nextMessage),
            renderList: () => {
                renderList();
            },
            setStatusError: params.setStatusError,
        });
        const renderPreview = () => {
            previewRenderOrchestrationUtils.renderPreview({
                findSelectedItem: params.findSelectedItem,
                getPreviewMessageEditor: () => params.state.previewMessageEditor,
                hasPendingPreviewMessageChanges: previewMessageHandlers.hasPendingPreviewMessageChanges,
                clearPreviewMessageAutosave: params.clearPreviewMessageAutosave,
                savePreviewMessage: previewMessageHandlers.savePreviewMessage,
                resetPreviewMessageEditor: params.resetPreviewMessageEditor,
                setPreviewMediaKey: (nextKey) => {
                    params.state.previewMediaKey = nextKey;
                },
                renderEmptyPreviewState: previewRenderStateUtils.renderEmptyPreviewState,
                previewStageNode: params.nodes.previewStageNode,
                selectedTitleNode: params.nodes.selectedTitleNode,
                selectedMetaNode: params.nodes.selectedMetaNode,
                createEmptyStateNode: renderUtils.createEmptyStateNode,
                refreshPreviewLayoutObserverTargets: params.refreshPreviewLayoutObserverTargets,
                shouldResetPreviewMessageEditor: previewRenderStateUtils.shouldResetPreviewMessageEditor,
                buildPreviewMediaContext: previewRenderStateUtils.buildPreviewMediaContext,
                getPreviewMediaKey: () => params.state.previewMediaKey,
                mountPreviewMediaIfNeeded: previewMountUtils.mountPreviewMediaIfNeeded,
                setPreviewMessageEditor: (nextEditor) => {
                    params.state.previewMessageEditor = nextEditor;
                },
                schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
                schedulePreviewMessageAutosave: previewMessageHandlers.schedulePreviewMessageAutosave,
                createPreviewMediaNode: renderUtils.createPreviewMediaNode,
                createPreviewMessageEditor: previewControlsUtils.createPreviewMessageEditor,
                createPreviewControls: previewControlsUtils.createPreviewControls,
                createPreviewControlHandlers: () => {
                    const actionHandlers = params.getActionHandlers();
                    return previewActionUtils.createPreviewControlHandlers({
                        findSelectedItem: params.findSelectedItem,
                        triggerItem: actionHandlers.triggerItem,
                        renameItem: actionHandlers.renameItem,
                        flushPreviewMessageChanges: previewMessageHandlers.flushPreviewMessageChanges,
                        beginCaptureForItem: actionHandlers.beginCaptureForItem,
                        clearShortcutForItem: actionHandlers.clearShortcutForItem,
                        deleteItem: actionHandlers.deleteItem,
                    });
                },
                applyPreviewVolume: params.applyPreviewVolume,
                getItemShortcuts: params.getItemShortcuts,
                toCardTitle: params.toCardTitle,
                toMessagePreview: params.toMessagePreview,
                buildPreviewMetaText: renderUtils.buildPreviewMetaText,
                normalizeMediaKind: params.normalizeMediaKind,
                buildAuthedUrl: params.buildAuthedUrl,
            });
        };
        renderList = listOrchestrationUtils.createRenderList({
            listActionUtils,
            listRenderUtils,
            getSelectedId: () => params.state.selectedId,
            flushPreviewMessageChanges: previewMessageHandlers.flushPreviewMessageChanges,
            clearStatus: params.clearStatus,
            selectItem: (item) => {
                params.state.selectedId = item?.id || null;
                params.state.selectedItem = item;
            },
            getItems: () => params.state.items,
            getTotal: () => params.state.total,
            getItemShortcuts: params.getItemShortcuts,
            toCardTitle: params.toCardTitle,
            toSafeDateLabel: params.toSafeDateLabel,
            toMessagePreview: params.toMessagePreview,
            createEmptyStateNode: renderUtils.createEmptyStateNode,
            createItemCard: renderUtils.createItemCard,
            triggerItem: async (itemId) => {
                await params.getActionHandlers().triggerItem(itemId);
            },
            renameItem: async (item) => {
                await params.getActionHandlers().renameItem(item);
            },
            beginCaptureForItem: (itemId) => {
                params.getActionHandlers().beginCaptureForItem(itemId);
            },
            deleteItem: async (item) => {
                await params.getActionHandlers().deleteItem(item);
            },
            renderPreview,
            itemsListNode: params.nodes.itemsListNode,
            countPillNode: params.nodes.countPillNode,
        });
        return {
            hasPendingPreviewMessageChanges: previewMessageHandlers.hasPendingPreviewMessageChanges,
            schedulePreviewMessageAutosave: previewMessageHandlers.schedulePreviewMessageAutosave,
            savePreviewMessage: (options) => previewMessageHandlers.savePreviewMessage(options),
            flushPreviewMessageChanges: previewMessageHandlers.flushPreviewMessageChanges,
            flushPreviewMessageOnLifecycleExit: previewMessageHandlers.flushPreviewMessageOnLifecycleExit,
            renderPreview,
            renderList,
        };
    }
    return {
        createRuntimePreviewListHandlers,
    };
}
