interface ListActionHandlers {
  onSelectItem: (item: unknown) => void;
  onTriggerItem: (itemId: string) => void;
  onRenameItem: (item: unknown) => void;
  onBindItem: (itemId: string) => void;
  onDeleteItem: (item: unknown) => void;
}

interface CreateListActionHandlersParams {
  getSelectedId: () => string | null;
  flushPreviewMessageChanges: () => Promise<boolean>;
  clearStatus: () => void;
  selectItem: (item: unknown) => void;
  renderList: () => void;
  triggerItem: (itemId: string) => Promise<void>;
  renameItem: (item: unknown) => Promise<void>;
  beginCaptureForItem: (itemId: string) => void;
  deleteItem: (item: unknown) => Promise<void>;
}

export interface BoardLegacyListActionUtils {
  createListActionHandlers(params: CreateListActionHandlersParams): ListActionHandlers;
}

function toItemId(value: unknown): string {
  if (typeof value !== 'object' || value === null) {
    return '';
  }

  const record = value as { id?: unknown };
  return `${record.id || ''}`.trim();
}

export function createBoardLegacyListActionUtils(): BoardLegacyListActionUtils {
  function createListActionHandlers(params: CreateListActionHandlersParams): ListActionHandlers {
    return {
      onSelectItem: (item) => {
        void (async () => {
          const nextItemId = toItemId(item);
          if (`${params.getSelectedId() || ''}` !== nextItemId) {
            const flushed = await params.flushPreviewMessageChanges();
            if (!flushed) {
              return;
            }
            params.clearStatus();
          }

          params.selectItem(item);
          params.renderList();
        })();
      },
      onTriggerItem: (itemId) => {
        void params.triggerItem(itemId);
      },
      onRenameItem: (item) => {
        void params.renameItem(item);
      },
      onBindItem: (itemId) => {
        void (async () => {
          const flushed = await params.flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }

          params.beginCaptureForItem(itemId);
        })();
      },
      onDeleteItem: (item) => {
        void params.deleteItem(item);
      },
    };
  }

  return {
    createListActionHandlers,
  };
}
