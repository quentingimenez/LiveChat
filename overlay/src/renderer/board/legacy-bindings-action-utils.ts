interface FilterBindingsResult {
  nextBindings: Record<string, string>;
  removedCount: number;
}

interface SyncBindingsWithBoardItemsParams {
  items: unknown;
  bindings: unknown;
  filterBindingsByValidItemIds: (bindings: unknown, validItemIds: Iterable<string>) => FilterBindingsResult;
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
}

interface RemoveBindingsResult {
  nextBindings: Record<string, string>;
  removedCount: number;
}

interface ClearShortcutForItemParams {
  itemId: string;
  flushPreviewMessageChanges: () => Promise<boolean>;
  removeBindingsForItem: (itemId: string) => RemoveBindingsResult;
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  renderList: () => void;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

export interface BoardLegacyBindingsActionUtils {
  syncBindingsWithBoardItems(params: SyncBindingsWithBoardItemsParams): Promise<void>;
  clearShortcutForItem(params: ClearShortcutForItemParams): Promise<void>;
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) {
    return error.message;
  }

  if (typeof error === 'string') {
    return error;
  }

  return `${error}`;
}

function collectValidItemIds(items: unknown): Set<string> {
  const values = Array.isArray(items) ? items : [];
  const validItemIds = new Set<string>();

  for (const item of values) {
    const itemId = `${(item as { id?: unknown } | null)?.id || ''}`.trim();
    if (itemId) {
      validItemIds.add(itemId);
    }
  }

  return validItemIds;
}

export function createBoardLegacyBindingsActionUtils(): BoardLegacyBindingsActionUtils {
  async function syncBindingsWithBoardItems(params: SyncBindingsWithBoardItemsParams): Promise<void> {
    const validItemIds = collectValidItemIds(params.items);
    const { nextBindings, removedCount } = params.filterBindingsByValidItemIds(params.bindings, validItemIds);

    if (removedCount === 0) {
      return;
    }

    await params.persistBindings(nextBindings);
  }

  async function clearShortcutForItem(params: ClearShortcutForItemParams): Promise<void> {
    const flushed = await params.flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const { nextBindings, removedCount } = params.removeBindingsForItem(params.itemId);

    if (removedCount === 0) {
      params.setStatusSuccess('Aucun raccourci à retirer pour ce mème.');
      return;
    }

    try {
      await params.persistBindings(nextBindings);
      params.renderList();
      params.setStatusSuccess('Raccourci retiré.');
    } catch (error) {
      params.setStatusError(toErrorMessage(error) || 'Impossible de retirer le raccourci.');
    }
  }

  return {
    syncBindingsWithBoardItems,
    clearShortcutForItem,
  };
}
