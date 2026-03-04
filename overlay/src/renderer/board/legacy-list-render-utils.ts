interface CreateEmptyStateNodeParams {
  text: string;
}

interface CreateItemCardParams {
  item: unknown;
  isSelected: boolean;
  shortcuts: string[];
  toCardTitle: (item: unknown) => string;
  toSafeDateLabel: (value: unknown) => string;
  toMessagePreview: (value: unknown, maxLength?: number) => string;
  onSelect: () => void;
  onTrigger: () => void;
  onRename: () => void;
  onBind: () => void;
  onDelete: () => void;
}

interface RenderItemListParams {
  itemsListNode: unknown;
  countPillNode: unknown;
  items: unknown;
  total: number;
  selectedId: unknown;
  getItemShortcuts: (itemId: string) => string[];
  toCardTitle: (item: unknown) => string;
  toSafeDateLabel: (value: unknown) => string;
  toMessagePreview: (value: unknown, maxLength?: number) => string;
  createEmptyStateNode: (params: CreateEmptyStateNodeParams) => Node;
  createItemCard: (params: CreateItemCardParams) => Node;
  onSelectItem: (item: unknown) => void;
  onTriggerItem: (itemId: string) => void;
  onRenameItem: (item: unknown) => void;
  onBindItem: (itemId: string) => void;
  onDeleteItem: (item: unknown) => void;
  onRenderPreview: () => void;
}

export interface BoardLegacyListRenderUtils {
  renderItemList(params: RenderItemListParams): void;
}

function toItemArray(items: unknown): unknown[] {
  return Array.isArray(items) ? items : [];
}

function toItemId(item: unknown): string {
  if (typeof item !== 'object' || item === null) {
    return '';
  }

  const record = item as { id?: unknown };
  return `${record.id || ''}`.trim();
}

export function createBoardLegacyListRenderUtils(): BoardLegacyListRenderUtils {
  function renderItemList(params: RenderItemListParams): void {
    const itemsListNode = params.itemsListNode;
    const countPillNode = params.countPillNode;

    if (itemsListNode instanceof HTMLElement) {
      itemsListNode.innerHTML = '';
    }

    if (countPillNode instanceof HTMLElement) {
      countPillNode.textContent = `${params.total}`;
    }

    const items = toItemArray(params.items);
    if (items.length === 0) {
      if (itemsListNode instanceof HTMLElement) {
        const emptyNode = params.createEmptyStateNode({
          text: 'Aucun mème dans la board pour cette recherche.',
        });
        itemsListNode.appendChild(emptyNode);
      }

      params.onRenderPreview();
      return;
    }

    const normalizedSelectedId = `${params.selectedId || ''}`;
    for (const item of items) {
      const itemId = toItemId(item);
      const shortcuts = params.getItemShortcuts(itemId);
      const card = params.createItemCard({
        item,
        isSelected: itemId === normalizedSelectedId,
        shortcuts,
        toCardTitle: params.toCardTitle,
        toSafeDateLabel: params.toSafeDateLabel,
        toMessagePreview: params.toMessagePreview,
        onSelect: () => {
          params.onSelectItem(item);
        },
        onTrigger: () => {
          params.onTriggerItem(itemId);
        },
        onRename: () => {
          params.onRenameItem(item);
        },
        onBind: () => {
          params.onBindItem(itemId);
        },
        onDelete: () => {
          params.onDeleteItem(item);
        },
      });

      if (itemsListNode instanceof HTMLElement) {
        itemsListNode.appendChild(card);
      }
    }

    params.onRenderPreview();
  }

  return {
    renderItemList,
  };
}
