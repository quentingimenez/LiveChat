interface ResolveSelectedItemForPreviewParams {
  items: unknown;
  selectedId: unknown;
  selectedItem: unknown;
}

interface ResolveSelectionAfterItemsLoadParams {
  items: unknown;
  selectedId: unknown;
  selectedItem: unknown;
}

interface BoardSelectionState {
  selectedId: string | null;
  selectedItem: unknown;
}

interface ResolveSelectedItemForPreviewResult {
  selectedItem: unknown;
  syncedSelectedItem: unknown;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function toItemArray(items: unknown): unknown[] {
  return Array.isArray(items) ? items : [];
}

function toItemId(item: unknown): string {
  const itemRecord = toRecord(item);
  return `${itemRecord.id || ''}`.trim();
}

function toSelectedId(selectedId: unknown): string | null {
  if (typeof selectedId !== 'string') {
    return null;
  }

  const normalized = selectedId.trim();
  return normalized ? normalized : null;
}

function findItemById(items: unknown[], targetId: string | null): unknown {
  if (!targetId) {
    return null;
  }

  return items.find((item) => toItemId(item) === targetId) || null;
}

export interface BoardLegacySelectionUtils {
  resolveSelectedItemForPreview(params: ResolveSelectedItemForPreviewParams): ResolveSelectedItemForPreviewResult;
  resolveSelectionAfterItemsLoad(params: ResolveSelectionAfterItemsLoadParams): BoardSelectionState;
}

export function createBoardLegacySelectionUtils(): BoardLegacySelectionUtils {
  function resolveSelectedItemForPreview(params: ResolveSelectedItemForPreviewParams): ResolveSelectedItemForPreviewResult {
    const items = toItemArray(params.items);
    const selectedId = toSelectedId(params.selectedId);

    const selectedFromList = findItemById(items, selectedId);
    if (selectedFromList) {
      return {
        selectedItem: selectedFromList,
        syncedSelectedItem: selectedFromList
      };
    }

    const currentSelectedItem = params.selectedItem;
    const currentSelectedItemId = toItemId(currentSelectedItem);

    if (currentSelectedItemId && (!selectedId || currentSelectedItemId === selectedId)) {
      return {
        selectedItem: currentSelectedItem,
        syncedSelectedItem: currentSelectedItem
      };
    }

    return {
      selectedItem: null,
      syncedSelectedItem: params.selectedItem
    };
  }

  function resolveSelectionAfterItemsLoad(params: ResolveSelectionAfterItemsLoadParams): BoardSelectionState {
    const items = toItemArray(params.items);
    const selectedId = toSelectedId(params.selectedId);
    const selectedItem = params.selectedItem;

    const selectedInList = findItemById(items, selectedId);
    if (selectedInList) {
      return {
        selectedId,
        selectedItem: selectedInList
      };
    }

    if (!selectedId && !selectedItem && items.length > 0) {
      const firstItem = items[0];
      const firstItemId = toItemId(firstItem);

      return {
        selectedId: firstItemId || null,
        selectedItem: firstItem
      };
    }

    return {
      selectedId,
      selectedItem
    };
  }

  return {
    resolveSelectedItemForPreview,
    resolveSelectionAfterItemsLoad
  };
}
