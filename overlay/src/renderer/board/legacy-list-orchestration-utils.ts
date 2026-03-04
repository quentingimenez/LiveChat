import type { BoardLegacyListActionUtils } from './legacy-list-action-utils';
import type { BoardLegacyListRenderUtils } from './legacy-list-render-utils';

interface CreateRenderListParams {
  listActionUtils: BoardLegacyListActionUtils;
  listRenderUtils: BoardLegacyListRenderUtils;
  getSelectedId: () => string | null;
  flushPreviewMessageChanges: () => Promise<boolean>;
  clearStatus: () => void;
  selectItem: (item: unknown) => void;
  getItems: () => unknown;
  getTotal: () => number;
  getItemShortcuts: (itemId: string) => string[];
  toCardTitle: (item: unknown) => string;
  toSafeDateLabel: (value: unknown) => string;
  toMessagePreview: (value: unknown, maxLength?: number) => string;
  createEmptyStateNode: (params: { text: string }) => Node;
  createItemCard: (params: unknown) => Node;
  triggerItem: (itemId: string) => Promise<void>;
  renameItem: (item: unknown) => Promise<void>;
  beginCaptureForItem: (itemId: string) => void;
  deleteItem: (item: unknown) => Promise<void>;
  renderPreview: () => void;
  itemsListNode: unknown;
  countPillNode: unknown;
}

export interface BoardLegacyListOrchestrationUtils {
  createRenderList(params: CreateRenderListParams): () => void;
}

export function createBoardLegacyListOrchestrationUtils(): BoardLegacyListOrchestrationUtils {
  function createRenderList(params: CreateRenderListParams): () => void {
    function renderList(): void {
      const listActionHandlers = params.listActionUtils.createListActionHandlers({
        getSelectedId: params.getSelectedId,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        clearStatus: params.clearStatus,
        selectItem: params.selectItem,
        renderList,
        triggerItem: params.triggerItem,
        renameItem: params.renameItem,
        beginCaptureForItem: params.beginCaptureForItem,
        deleteItem: params.deleteItem,
      });

      params.listRenderUtils.renderItemList({
        itemsListNode: params.itemsListNode,
        countPillNode: params.countPillNode,
        items: params.getItems(),
        total: params.getTotal(),
        selectedId: params.getSelectedId(),
        getItemShortcuts: params.getItemShortcuts,
        toCardTitle: params.toCardTitle,
        toSafeDateLabel: params.toSafeDateLabel,
        toMessagePreview: params.toMessagePreview,
        createEmptyStateNode: params.createEmptyStateNode,
        createItemCard: params.createItemCard,
        onSelectItem: listActionHandlers.onSelectItem,
        onTriggerItem: listActionHandlers.onTriggerItem,
        onRenameItem: listActionHandlers.onRenameItem,
        onBindItem: listActionHandlers.onBindItem,
        onDeleteItem: listActionHandlers.onDeleteItem,
        onRenderPreview: params.renderPreview,
      });
    }

    return renderList;
  }

  return {
    createRenderList,
  };
}
