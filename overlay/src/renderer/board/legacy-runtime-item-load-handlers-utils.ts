import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types.js';

interface CreateRuntimeItemLoadHandlersParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  clearSearchDebounce: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  clearStatus: () => void;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
  renderList: () => void;
  openRenameDialog: (title: string, message: string) => Promise<unknown>;
  openDeleteDialog: (item: unknown) => Promise<unknown>;
  instantSearchDebounceMs: number;
  setTimeout: (callback: () => void, delayMs: number) => number;
  setMemeBindings: (payload: unknown) => Promise<unknown>;
  triggerMeme: (payload: unknown) => Promise<unknown>;
  stopMemePlayback: () => Promise<unknown>;
}

interface RuntimeItemLoadHandlers {
  patchItemMetadata: (itemId: string, payload: { title: string; message: string }, options?: unknown) => Promise<void>;
  applyLocalItemMetadata: (itemId: string, nextTitle: string, nextMessage: string) => boolean;
  loadItemsAndRender: () => Promise<void>;
  runSearchNow: () => Promise<void>;
  scheduleInstantSearch: () => void;
  addItemFromLink: (payload: { url: string; title: string; message: string; forceRefresh: boolean }) => Promise<void>;
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  triggerItem: (itemId: string) => Promise<void>;
  stopCurrentPlayback: () => Promise<void>;
  syncBindingsWithBoardItems: () => Promise<void>;
  clearShortcutForItem: (itemId: string) => Promise<void>;
  renameItem: (item: unknown) => Promise<void>;
  deleteItem: (item: unknown) => Promise<void>;
}

export interface BoardLegacyRuntimeItemLoadHandlersUtils {
  createRuntimeItemLoadHandlers(params: CreateRuntimeItemLoadHandlersParams): RuntimeItemLoadHandlers;
}

function requireUtility<T>(utility: T | undefined, utilityName: string): T {
  if (utility === undefined || utility === null) {
    throw new Error(`Board legacy utility unavailable: ${utilityName}`);
  }

  return utility;
}

export function createBoardLegacyRuntimeItemLoadHandlersUtils(): BoardLegacyRuntimeItemLoadHandlersUtils {
  function createRuntimeItemLoadHandlers(params: CreateRuntimeItemLoadHandlersParams): RuntimeItemLoadHandlers {
    const itemUtils = requireUtility(window.__boardLegacyItemUtils, 'item');
    const loadSearchUtils = requireUtility(window.__boardLegacyLoadSearchUtils, 'load-search');
    const searchUtils = requireUtility(window.__boardLegacySearchUtils, 'search');
    const selectionUtils = requireUtility(window.__boardLegacySelectionUtils, 'selection');
    const loadSearchOrchestrationUtils = requireUtility(window.__boardLegacyLoadSearchOrchestrationUtils, 'load-search-orchestration');
    const itemActionUtils = requireUtility(window.__boardLegacyItemActionUtils, 'item-action');
    const itemMetadataActionsUtils = requireUtility(window.__boardLegacyItemMetadataActionsUtils, 'item-metadata-action');
    const bindingsUtils = requireUtility(window.__boardLegacyBindingsUtils, 'bindings');
    const bindingsActionUtils = requireUtility(window.__boardLegacyBindingsActionUtils, 'bindings-action');
    const itemActionsOrchestrationUtils = requireUtility(window.__boardLegacyItemActionsOrchestrationUtils, 'item-actions-orchestration');

    let syncBindingsWithBoardItems: () => Promise<void> = async () => {};

    const loadSearchHandlers = loadSearchOrchestrationUtils.createLoadSearchHandlers({
      itemUtils,
      loadSearchUtils,
      searchUtils,
      state: params.state,
      buildAuthedUrl: params.buildAuthedUrl,
      getSearchInputValue: () => `${(params.nodes.searchInput as HTMLInputElement | null)?.value || ''}`,
      clearSearchDebounce: params.clearSearchDebounce,
      flushPreviewMessageChanges: params.flushPreviewMessageChanges,
      resolveSelectionAfterItemsLoad: selectionUtils.resolveSelectionAfterItemsLoad,
      syncBindingsWithBoardItems: async () => {
        await syncBindingsWithBoardItems();
      },
      renderList: params.renderList,
      clearStatus: params.clearStatus,
      setStatusError: params.setStatusError,
      scheduleDebouncedTask: searchUtils.scheduleDebouncedTask,
      instantSearchDebounceMs: params.instantSearchDebounceMs,
      setTimeout: params.setTimeout,
      setItems: (nextItems) => {
        params.state.items = nextItems;
      },
      setSelectedItem: (nextSelectedItem) => {
        params.state.selectedItem = nextSelectedItem;
      },
    });

    const itemActionHandlers = itemActionsOrchestrationUtils.createItemActionHandlers({
      itemUtils,
      itemActionUtils,
      itemMetadataActionsUtils,
      bindingsUtils,
      bindingsActionUtils,
      buildAuthedUrl: params.buildAuthedUrl,
      getBindings: () => params.state.bindings,
      setBindings: (nextBindings) => {
        params.state.bindings = nextBindings;
      },
      getItems: () => params.state.items,
      getSelectedId: () => params.state.selectedId,
      setSelectedId: (nextSelectedId) => {
        params.state.selectedId = nextSelectedId;
      },
      setSelectedItem: (nextSelectedItem) => {
        params.state.selectedItem = nextSelectedItem;
      },
      setPreviewMediaKey: (nextPreviewMediaKey) => {
        params.state.previewMediaKey = nextPreviewMediaKey;
      },
      flushPreviewMessageChanges: params.flushPreviewMessageChanges,
      loadItemsAndRender: loadSearchHandlers.loadItemsAndRender,
      openRenameDialog: params.openRenameDialog as Parameters<
        (typeof itemActionsOrchestrationUtils)['createItemActionHandlers']
      >[0]['openRenameDialog'],
      openDeleteDialog: params.openDeleteDialog as Parameters<
        (typeof itemActionsOrchestrationUtils)['createItemActionHandlers']
      >[0]['openDeleteDialog'],
      patchItemMetadata: loadSearchHandlers.patchItemMetadata,
      renderList: params.renderList,
      setStatusSuccess: params.setStatusSuccess,
      setStatusError: params.setStatusError,
      setMemeBindings: ((payload) => params.setMemeBindings(payload)) as Parameters<
        (typeof itemActionsOrchestrationUtils)['createItemActionHandlers']
      >[0]['setMemeBindings'],
      triggerMeme: ((payload) => params.triggerMeme(payload)) as Parameters<
        (typeof itemActionsOrchestrationUtils)['createItemActionHandlers']
      >[0]['triggerMeme'],
      stopMemePlayback: (() => params.stopMemePlayback()) as Parameters<
        (typeof itemActionsOrchestrationUtils)['createItemActionHandlers']
      >[0]['stopMemePlayback'],
    });

    syncBindingsWithBoardItems = itemActionHandlers.syncBindingsWithBoardItems;

    return {
      patchItemMetadata: loadSearchHandlers.patchItemMetadata,
      applyLocalItemMetadata: loadSearchHandlers.applyLocalItemMetadata,
      loadItemsAndRender: loadSearchHandlers.loadItemsAndRender,
      runSearchNow: loadSearchHandlers.runSearchNow,
      scheduleInstantSearch: loadSearchHandlers.scheduleInstantSearch,
      addItemFromLink: itemActionHandlers.addItemFromLink,
      persistBindings: itemActionHandlers.persistBindings,
      triggerItem: itemActionHandlers.triggerItem,
      stopCurrentPlayback: itemActionHandlers.stopCurrentPlayback,
      syncBindingsWithBoardItems: itemActionHandlers.syncBindingsWithBoardItems,
      clearShortcutForItem: itemActionHandlers.clearShortcutForItem,
      renameItem: itemActionHandlers.renameItem,
      deleteItem: itemActionHandlers.deleteItem,
    } as RuntimeItemLoadHandlers;
  }

  return {
    createRuntimeItemLoadHandlers,
  };
}
