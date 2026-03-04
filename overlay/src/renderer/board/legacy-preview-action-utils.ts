interface PreviewControlHandlers {
  onPlay: () => void;
  onRename: () => void;
  onBindShortcut: () => void;
  onClearShortcut: () => void;
  onDelete: () => void;
}

interface CreatePreviewControlHandlersParams {
  findSelectedItem: () => unknown;
  triggerItem: (itemId: string) => Promise<void>;
  renameItem: (item: unknown) => Promise<void>;
  flushPreviewMessageChanges: () => Promise<boolean>;
  beginCaptureForItem: (itemId: string) => void;
  clearShortcutForItem: (itemId: string) => Promise<void>;
  deleteItem: (item: unknown) => Promise<void>;
}

export interface BoardLegacyPreviewActionUtils {
  createPreviewControlHandlers(params: CreatePreviewControlHandlersParams): PreviewControlHandlers;
}

function toSelectedItem(item: unknown): { id: string } | null {
  if (typeof item !== 'object' || item === null) {
    return null;
  }

  const record = item as { id?: unknown };
  const itemId = `${record.id || ''}`.trim();
  if (!itemId) {
    return null;
  }

  return { id: itemId };
}

export function createBoardLegacyPreviewActionUtils(): BoardLegacyPreviewActionUtils {
  function createPreviewControlHandlers(params: CreatePreviewControlHandlersParams): PreviewControlHandlers {
    return {
      onPlay: () => {
        void (async () => {
          const selectedItem = params.findSelectedItem();
          const selected = toSelectedItem(selectedItem);
          if (!selected) {
            return;
          }

          await params.triggerItem(selected.id);
        })();
      },
      onRename: () => {
        void (async () => {
          const selectedItem = params.findSelectedItem();
          if (!selectedItem) {
            return;
          }

          await params.renameItem(selectedItem);
        })();
      },
      onBindShortcut: () => {
        void (async () => {
          const selectedItem = params.findSelectedItem();
          const selected = toSelectedItem(selectedItem);
          if (!selected) {
            return;
          }

          const flushed = await params.flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }

          params.beginCaptureForItem(selected.id);
        })();
      },
      onClearShortcut: () => {
        void (async () => {
          const selectedItem = params.findSelectedItem();
          const selected = toSelectedItem(selectedItem);
          if (!selected) {
            return;
          }

          await params.clearShortcutForItem(selected.id);
        })();
      },
      onDelete: () => {
        void (async () => {
          const selectedItem = params.findSelectedItem();
          if (!selectedItem) {
            return;
          }

          await params.deleteItem(selectedItem);
        })();
      },
    };
  }

  return {
    createPreviewControlHandlers,
  };
}
