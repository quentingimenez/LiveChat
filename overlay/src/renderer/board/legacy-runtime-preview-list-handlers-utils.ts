import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types.js';

interface RuntimeActionHandlers {
  triggerItem: (itemId: unknown) => Promise<void>;
  renameItem: (item: unknown) => Promise<void>;
  beginCaptureForItem: (itemId: unknown) => void;
  clearShortcutForItem: (itemId: unknown) => Promise<void>;
  deleteItem: (item: unknown) => Promise<void>;
}

interface CreateRuntimePreviewListHandlersParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  defaultAutosaveDelayMs: number;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearTimeout: (timeoutId: number) => void;
  clearPreviewMessageAutosave: () => void;
  resetPreviewMessageEditor: () => void;
  schedulePreviewLayoutRefresh: () => void;
  refreshPreviewLayoutObserverTargets: () => void;
  clearStatus: () => void;
  setStatusError: (message: string) => void;
  findSelectedItem: () => unknown;
  getItemShortcuts: (itemId: unknown) => string[];
  toCardTitle: (item: unknown) => string;
  toSafeDateLabel: (value: unknown) => string;
  toMessagePreview: (value: unknown, maxLength?: number) => string;
  normalizeMediaKind: (value: unknown) => string;
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  applyPreviewVolume: (root?: unknown) => void;
  resolvePreviewMessageItemTitle: (itemId: unknown) => string;
  getPatchItemMetadata: () => (itemId: unknown, payload: unknown, options?: unknown) => Promise<unknown>;
  getApplyLocalItemMetadata: () => (itemId: unknown, nextTitle: unknown, nextMessage: unknown) => unknown;
  getActionHandlers: () => RuntimeActionHandlers;
}

interface RuntimePreviewListHandlers {
  hasPendingPreviewMessageChanges: () => boolean;
  schedulePreviewMessageAutosave: () => void;
  savePreviewMessage: (options?: unknown) => Promise<boolean>;
  flushPreviewMessageChanges: () => Promise<boolean>;
  flushPreviewMessageOnLifecycleExit: () => void;
  renderPreview: () => void;
  renderList: () => void;
}

export interface BoardLegacyRuntimePreviewListHandlersUtils {
  createRuntimePreviewListHandlers(params: CreateRuntimePreviewListHandlersParams): RuntimePreviewListHandlers;
}

function requireUtility<T>(utility: T | undefined, utilityName: string): T {
  if (utility === undefined || utility === null) {
    throw new Error(`Board legacy utility unavailable: ${utilityName}`);
  }

  return utility;
}

export function createBoardLegacyRuntimePreviewListHandlersUtils(): BoardLegacyRuntimePreviewListHandlersUtils {
  function createRuntimePreviewListHandlers(params: CreateRuntimePreviewListHandlersParams): RuntimePreviewListHandlers {
    const previewMessageUtils = requireUtility(window.__boardLegacyPreviewMessageUtils, 'preview-message');
    const previewMessageOrchestrationUtils = requireUtility(
      window.__boardLegacyPreviewMessageOrchestrationUtils,
      'preview-message-orchestration'
    );
    const previewRenderOrchestrationUtils = requireUtility(
      window.__boardLegacyPreviewRenderOrchestrationUtils,
      'preview-render-orchestration'
    );
    const previewRenderStateUtils = requireUtility(window.__boardLegacyPreviewRenderStateUtils, 'preview-render-state');
    const previewMountUtils = requireUtility(window.__boardLegacyPreviewMountUtils, 'preview-mount');
    const previewControlsUtils = requireUtility(window.__boardLegacyPreviewControlsUtils, 'preview-controls');
    const previewActionUtils = requireUtility(window.__boardLegacyPreviewActionUtils, 'preview-action');
    const renderUtils = requireUtility(window.__boardLegacyRenderUtils, 'render');
    const listActionUtils = requireUtility(window.__boardLegacyListActionUtils, 'list-action');
    const listRenderUtils = requireUtility(window.__boardLegacyListRenderUtils, 'list-render');
    const listOrchestrationUtils = requireUtility(window.__boardLegacyListOrchestrationUtils, 'list-orchestration');

    let renderList = () => {};

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
      applyLocalItemMetadata: (itemId, nextTitle, nextMessage) =>
        params.getApplyLocalItemMetadata()(itemId, nextTitle, nextMessage),
      renderList: () => {
        renderList();
      },
      setStatusError: params.setStatusError,
    } as Parameters<(typeof previewMessageOrchestrationUtils)['createPreviewMessageHandlers']>[0]);

    const renderPreview = () => {
      previewRenderOrchestrationUtils.renderPreview({
        findSelectedItem: params.findSelectedItem,
        getPreviewMessageEditor: () => params.state.previewMessageEditor,
        hasPendingPreviewMessageChanges: previewMessageHandlers.hasPendingPreviewMessageChanges,
        clearPreviewMessageAutosave: params.clearPreviewMessageAutosave,
        savePreviewMessage: previewMessageHandlers.savePreviewMessage,
        resetPreviewMessageEditor: params.resetPreviewMessageEditor,
        setPreviewMediaKey: (nextKey: string | null) => {
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
        setPreviewMessageEditor: (nextEditor: unknown) => {
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
      } as unknown as Parameters<(typeof previewRenderOrchestrationUtils)['renderPreview']>[0]);
    };

    renderList = listOrchestrationUtils.createRenderList({
      listActionUtils,
      listRenderUtils,
      getSelectedId: () => params.state.selectedId,
      flushPreviewMessageChanges: previewMessageHandlers.flushPreviewMessageChanges,
      clearStatus: params.clearStatus,
      selectItem: (item) => {
        params.state.selectedId = (item as { id?: string | null } | null)?.id || null;
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
    } as Parameters<(typeof listOrchestrationUtils)['createRenderList']>[0]);

    return {
      hasPendingPreviewMessageChanges: previewMessageHandlers.hasPendingPreviewMessageChanges,
      schedulePreviewMessageAutosave: previewMessageHandlers.schedulePreviewMessageAutosave,
      savePreviewMessage: (options) =>
        previewMessageHandlers.savePreviewMessage(
          options as Parameters<(typeof previewMessageHandlers)['savePreviewMessage']>[0]
        ),
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
