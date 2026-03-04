import type { BoardLegacyItemUtils } from './legacy-item-utils';
import type { BoardLegacyLoadSearchUtils } from './legacy-load-search-utils';
import type { BoardLegacySearchUtils } from './legacy-search-utils';

interface PatchItemMetadataPayload {
  title: string;
  message: string;
}

interface PatchItemMetadataOptions {
  keepalive?: boolean;
}

interface CreateLoadSearchHandlersParams {
  itemUtils: BoardLegacyItemUtils;
  loadSearchUtils: BoardLegacyLoadSearchUtils;
  searchUtils: BoardLegacySearchUtils;
  state: {
    search: string;
    itemsLoadRequestId: number;
    items: unknown[];
    total: number;
    selectedId: string | null;
    selectedItem: unknown;
    searchDebounceTimeoutId: number | null;
  };
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  getSearchInputValue: () => string;
  clearSearchDebounce: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  resolveSelectionAfterItemsLoad: (params: {
    items: unknown;
    selectedId: unknown;
    selectedItem: unknown;
  }) => {
    selectedId: string | null;
    selectedItem: unknown;
  };
  syncBindingsWithBoardItems: () => Promise<void>;
  renderList: () => void;
  clearStatus: () => void;
  setStatusError: (message: string) => void;
  scheduleDebouncedTask: (params: {
    delayMs: number;
    setTimer: (callback: () => void, delayMs: number) => number;
    runTask: () => void;
  }) => number;
  instantSearchDebounceMs: number;
  setTimeout: (callback: () => void, delayMs: number) => number;
  setItems: (nextItems: unknown[]) => void;
  setSelectedItem: (nextSelectedItem: unknown) => void;
}

interface LoadSearchHandlers {
  fetchItems(searchQuery: string): Promise<{ items: unknown[]; total: number }>;
  patchItemMetadata(itemId: string, payload: PatchItemMetadataPayload, options?: PatchItemMetadataOptions): Promise<void>;
  applyLocalItemMetadata(itemId: string, nextTitle: string, nextMessage: string): boolean;
  loadItemsAndRender(): Promise<void>;
  runSearchNow(): Promise<void>;
  scheduleInstantSearch(): void;
}

export interface BoardLegacyLoadSearchOrchestrationUtils {
  createLoadSearchHandlers(params: CreateLoadSearchHandlersParams): LoadSearchHandlers;
}

export function createBoardLegacyLoadSearchOrchestrationUtils(): BoardLegacyLoadSearchOrchestrationUtils {
  function createLoadSearchHandlers(params: CreateLoadSearchHandlersParams): LoadSearchHandlers {
    async function fetchItems(searchQuery: string): Promise<{ items: unknown[]; total: number }> {
      return params.itemUtils.fetchItems({
        buildAuthedUrl: params.buildAuthedUrl,
        searchQuery,
      });
    }

    async function patchItemMetadata(
      itemId: string,
      payload: PatchItemMetadataPayload,
      options: PatchItemMetadataOptions = {},
    ): Promise<void> {
      await params.itemUtils.patchItemMetadata({
        buildAuthedUrl: params.buildAuthedUrl,
        itemId,
        payload,
        options,
      });
    }

    function applyLocalItemMetadata(itemId: string, nextTitle: string, nextMessage: string): boolean {
      const result = params.itemUtils.applyLocalItemMetadata({
        items: params.state.items,
        selectedItem: params.state.selectedItem,
        itemId,
        nextTitle,
        nextMessage,
      });

      params.setItems(result.items);
      params.setSelectedItem(result.selectedItem);
      return result.updated;
    }

    async function loadItemsAndRender(): Promise<void> {
      await params.loadSearchUtils.loadItemsAndRender({
        state: params.state,
        fetchItems,
        resolveSelectionAfterItemsLoad: params.resolveSelectionAfterItemsLoad,
        syncBindingsWithBoardItems: params.syncBindingsWithBoardItems,
        renderList: params.renderList,
        clearStatus: params.clearStatus,
        setStatusError: params.setStatusError,
      });
    }

    async function runSearchNow(): Promise<void> {
      await params.loadSearchUtils.runSearchNow({
        state: params.state,
        searchInputValue: params.getSearchInputValue(),
        clearSearchDebounce: params.clearSearchDebounce,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        normalizeSearchQuery: params.searchUtils.normalizeSearchQuery,
        loadItemsAndRender,
      });
    }

    function scheduleInstantSearch(): void {
      params.loadSearchUtils.scheduleInstantSearch({
        state: params.state,
        getSearchInputValue: params.getSearchInputValue,
        clearSearchDebounce: params.clearSearchDebounce,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        normalizeSearchQuery: params.searchUtils.normalizeSearchQuery,
        loadItemsAndRender,
        scheduleDebouncedTask: params.scheduleDebouncedTask,
        delayMs: params.instantSearchDebounceMs,
        setTimer: params.setTimeout,
      });
    }

    return {
      fetchItems,
      patchItemMetadata,
      applyLocalItemMetadata,
      loadItemsAndRender,
      runSearchNow,
      scheduleInstantSearch,
    };
  }

  return {
    createLoadSearchHandlers,
  };
}
