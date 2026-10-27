interface BoardLoadSearchState {
  search: string;
  searchDebounceTimeoutId: number | null;
  itemsLoadRequestId: number;
  items: unknown[];
  total: number;
  selectedId: string | null;
  selectedItem: unknown;
}

interface FetchItemsResult {
  items: unknown[];
  total: number;
}

interface ResolveSelectionAfterItemsLoadResult {
  selectedId: string | null;
  selectedItem: unknown;
}

interface LoadItemsAndRenderParams {
  state: BoardLoadSearchState;
  fetchItems: (searchQuery: string) => Promise<FetchItemsResult>;
  resolveSelectionAfterItemsLoad: (params: {
    items: unknown;
    selectedId: unknown;
    selectedItem: unknown;
  }) => ResolveSelectionAfterItemsLoadResult;
  syncBindingsWithBoardItems: () => Promise<void>;
  renderList: () => void;
  clearStatus: () => void;
  setStatusError: (message: string) => void;
}

interface RunSearchNowParams {
  state: BoardLoadSearchState;
  searchInputValue: string;
  clearSearchDebounce: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  normalizeSearchQuery: (value: string) => string;
  loadItemsAndRender: () => Promise<void>;
}

interface ScheduleDebouncedTaskOptions {
  delayMs: number;
  setTimer: (callback: () => void, delayMs: number) => number;
  runTask: () => void;
}

interface ScheduleInstantSearchParams {
  state: BoardLoadSearchState;
  getSearchInputValue: () => string;
  clearSearchDebounce: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  normalizeSearchQuery: (value: string) => string;
  loadItemsAndRender: () => Promise<void>;
  scheduleDebouncedTask: (options: ScheduleDebouncedTaskOptions) => number;
  delayMs: number;
  setTimer: (callback: () => void, delayMs: number) => number;
}

export interface BoardLegacyLoadSearchUtils {
  loadItemsAndRender(params: LoadItemsAndRenderParams): Promise<void>;
  runSearchNow(params: RunSearchNowParams): Promise<void>;
  scheduleInstantSearch(params: ScheduleInstantSearchParams): void;
}

export function createBoardLegacyLoadSearchUtils(): BoardLegacyLoadSearchUtils {
  async function loadItemsAndRender(params: LoadItemsAndRenderParams): Promise<void> {
    const requestId = params.state.itemsLoadRequestId + 1;
    params.state.itemsLoadRequestId = requestId;

    try {
      const fetched = await params.fetchItems(params.state.search);
      if (requestId !== params.state.itemsLoadRequestId) {
        return;
      }

      params.state.items = fetched.items;
      params.state.total = fetched.total;

      const nextSelection = params.resolveSelectionAfterItemsLoad({
        items: params.state.items,
        selectedId: params.state.selectedId,
        selectedItem: params.state.selectedItem,
      });

      params.state.selectedId = nextSelection.selectedId;
      params.state.selectedItem = nextSelection.selectedItem;

      await params.syncBindingsWithBoardItems();
      if (requestId !== params.state.itemsLoadRequestId) {
        return;
      }

      params.renderList();
      params.clearStatus();
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : '';
      params.setStatusError(errorMessage || 'Erreur de chargement de la mème board.');
    }
  }

  async function runSearchNow(params: RunSearchNowParams): Promise<void> {
    params.clearSearchDebounce();

    const flushed = await params.flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    params.state.search = params.normalizeSearchQuery(params.searchInputValue);
    await params.loadItemsAndRender();
  }

  function scheduleInstantSearch(params: ScheduleInstantSearchParams): void {
    params.clearSearchDebounce();

    params.state.searchDebounceTimeoutId = params.scheduleDebouncedTask({
      delayMs: params.delayMs,
      setTimer: params.setTimer,
      runTask: () => {
        params.state.searchDebounceTimeoutId = null;
        void (async () => {
          const flushed = await params.flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }

          params.state.search = params.normalizeSearchQuery(params.getSearchInputValue());
          await params.loadItemsAndRender();
        })();
      },
    });
  }

  return {
    loadItemsAndRender,
    runSearchNow,
    scheduleInstantSearch,
  };
}
