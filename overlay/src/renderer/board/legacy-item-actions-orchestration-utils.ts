import type { BoardLegacyBindingsActionUtils } from './legacy-bindings-action-utils.js';
import type { BoardLegacyBindingsUtils } from './legacy-bindings-utils.js';
import type { BoardLegacyItemActionUtils } from './legacy-item-action-utils.js';
import type { BoardLegacyItemMetadataActionsUtils } from './legacy-item-metadata-actions-utils.js';
import type { BoardLegacyItemUtils } from './legacy-item-utils.js';

interface MemeBindingsResult {
  ok?: boolean;
  bindings?: Record<string, string>;
  failedAccelerators?: string[];
}

interface ItemActionHandlers {
  addItemFromLink(payload: {
    url: string;
    title: string;
    message: string;
    forceRefresh: boolean;
  }): Promise<void>;
  persistBindings(nextBindings: Record<string, string>): Promise<void>;
  triggerItem(itemId: string): Promise<void>;
  stopCurrentPlayback(): Promise<void>;
  removeBindingsForItem(itemId: string): {
    nextBindings: Record<string, string>;
    removedCount: number;
  };
  syncBindingsWithBoardItems(): Promise<void>;
  clearShortcutForItem(itemId: string): Promise<void>;
  renameItem(item: unknown): Promise<void>;
  deleteItem(item: unknown): Promise<void>;
}

interface CreateItemActionHandlersParams {
  itemUtils: BoardLegacyItemUtils;
  itemActionUtils: BoardLegacyItemActionUtils;
  itemMetadataActionsUtils: BoardLegacyItemMetadataActionsUtils;
  bindingsUtils: BoardLegacyBindingsUtils;
  bindingsActionUtils: BoardLegacyBindingsActionUtils;
  buildAuthedUrl: (pathname: string, options?: Record<string, unknown>) => URL;
  getBindings: () => unknown;
  setBindings: (nextBindings: Record<string, string>) => void;
  getItems: () => unknown;
  getSelectedId: () => unknown;
  setSelectedId: (nextSelectedId: string | null) => void;
  setSelectedItem: (nextSelectedItem: unknown) => void;
  setPreviewMediaKey: (nextPreviewMediaKey: string | null) => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  loadItemsAndRender: () => Promise<void>;
  openRenameDialog: (title: string, message: string) => Promise<{ title: string; message: string } | null>;
  openDeleteDialog: (item: unknown) => Promise<boolean>;
  patchItemMetadata: (itemId: string, payload: { title: string; message: string }) => Promise<void>;
  renderList: () => void;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
  setMemeBindings: (payload: { bindings: Record<string, string> }) => Promise<MemeBindingsResult>;
  triggerMeme: (payload: { itemId: string; trigger: 'ui' }) => Promise<{ ok?: boolean; reason?: string }>;
  stopMemePlayback: () => Promise<{ ok?: boolean; reason?: string }>;
}

export interface BoardLegacyItemActionsOrchestrationUtils {
  createItemActionHandlers(params: CreateItemActionHandlersParams): ItemActionHandlers;
}

export function createBoardLegacyItemActionsOrchestrationUtils(): BoardLegacyItemActionsOrchestrationUtils {
  function createItemActionHandlers(params: CreateItemActionHandlersParams): ItemActionHandlers {
    async function persistBindings(nextBindings: Record<string, string>): Promise<void> {
      const result = await params.setMemeBindings({
        bindings: nextBindings,
      });

      if (!result || result.ok !== true) {
        const failed =
          Array.isArray(result?.failedAccelerators) && result.failedAccelerators.length > 0
            ? ` (${result.failedAccelerators.join(', ')})`
            : '';
        throw new Error(`Impossible d'appliquer les raccourcis${failed}`);
      }

      params.setBindings(result.bindings || {});
    }

    async function addItemFromLink(payload: {
      url: string;
      title: string;
      message: string;
      forceRefresh: boolean;
    }): Promise<void> {
      await params.itemActionUtils.addItemFromLink({
        payload,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
        createItem: (nextPayload) =>
          params.itemUtils.createItem({
            buildAuthedUrl: params.buildAuthedUrl,
            payload: nextPayload,
          }),
        onItemSelected: (item) => {
          if (!item || typeof item !== 'object') {
            return;
          }

          const itemRecord = item as { id?: unknown };
          if (!itemRecord.id) {
            return;
          }

          params.setSelectedId(`${itemRecord.id}`);
          params.setSelectedItem(item);
        },
        loadItemsAndRender: params.loadItemsAndRender,
      });
    }

    async function triggerItem(itemId: string): Promise<void> {
      await params.itemActionUtils.triggerItem({
        itemId,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        triggerMeme: async (nextItemId) =>
          params.triggerMeme({
            itemId: nextItemId,
            trigger: 'ui',
          }),
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
      });
    }

    async function stopCurrentPlayback(): Promise<void> {
      await params.itemActionUtils.stopCurrentPlayback({
        stopMemePlayback: params.stopMemePlayback,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
      });
    }

    function removeBindingsForItem(itemId: string): {
      nextBindings: Record<string, string>;
      removedCount: number;
    } {
      return params.bindingsUtils.removeBindingsForItem(params.getBindings(), itemId);
    }

    async function syncBindingsWithBoardItems(): Promise<void> {
      await params.bindingsActionUtils.syncBindingsWithBoardItems({
        items: params.getItems(),
        bindings: params.getBindings(),
        filterBindingsByValidItemIds: params.bindingsUtils.filterBindingsByValidItemIds,
        persistBindings,
      });
    }

    async function clearShortcutForItem(itemId: string): Promise<void> {
      await params.bindingsActionUtils.clearShortcutForItem({
        itemId,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        removeBindingsForItem,
        persistBindings,
        renderList: params.renderList,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
      });
    }

    async function renameItem(item: unknown): Promise<void> {
      await params.itemMetadataActionsUtils.renameItem({
        item,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        openRenameDialog: params.openRenameDialog,
        patchItemMetadata: params.patchItemMetadata,
        loadItemsAndRender: params.loadItemsAndRender,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
      });
    }

    async function deleteItem(item: unknown): Promise<void> {
      await params.itemMetadataActionsUtils.deleteItem({
        item,
        flushPreviewMessageChanges: params.flushPreviewMessageChanges,
        openDeleteDialog: params.openDeleteDialog,
        deleteItemById: (itemId) =>
          params.itemUtils.deleteItem({
            buildAuthedUrl: params.buildAuthedUrl,
            itemId,
          }),
        clearSelectionIfMatches: (itemId) => {
          if (`${params.getSelectedId() || ''}` === `${itemId || ''}`) {
            params.setSelectedId(null);
            params.setSelectedItem(null);
            params.setPreviewMediaKey(null);
          }
        },
        removeBindingsForItem,
        persistBindings,
        loadItemsAndRender: params.loadItemsAndRender,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: params.setStatusError,
      });
    }

    return {
      addItemFromLink,
      persistBindings,
      triggerItem,
      stopCurrentPlayback,
      removeBindingsForItem,
      syncBindingsWithBoardItems,
      clearShortcutForItem,
      renameItem,
      deleteItem,
    };
  }

  return {
    createItemActionHandlers,
  };
}
