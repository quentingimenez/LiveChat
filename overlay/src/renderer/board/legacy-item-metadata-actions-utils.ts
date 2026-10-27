interface RenameDialogResult {
  title: string;
  message: string;
}

interface RenameItemParams {
  item: unknown;
  flushPreviewMessageChanges: () => Promise<boolean>;
  openRenameDialog: (title: string, message: string) => Promise<RenameDialogResult | null>;
  patchItemMetadata: (itemId: string, payload: { title: string; message: string }) => Promise<void>;
  loadItemsAndRender: () => Promise<void>;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

interface RemoveBindingsResult {
  nextBindings: Record<string, string>;
  removedCount: number;
}

interface DeleteItemParams {
  item: unknown;
  flushPreviewMessageChanges: () => Promise<boolean>;
  openDeleteDialog: (item: unknown) => Promise<boolean>;
  deleteItemById: (itemId: string) => Promise<void>;
  clearSelectionIfMatches: (itemId: string) => void;
  removeBindingsForItem: (itemId: string) => RemoveBindingsResult;
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  loadItemsAndRender: () => Promise<void>;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

export interface BoardLegacyItemMetadataActionsUtils {
  renameItem(params: RenameItemParams): Promise<void>;
  deleteItem(params: DeleteItemParams): Promise<void>;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
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

function toItemId(item: unknown): string {
  if (typeof item !== 'object' || item === null) {
    return '';
  }

  const itemRecord = item as { id?: unknown };
  return toTrimmedString(itemRecord.id);
}

function toItemTitle(item: unknown): string {
  if (typeof item !== 'object' || item === null) {
    return '';
  }

  const itemRecord = item as { title?: unknown };
  return toTrimmedString(itemRecord.title);
}

function toItemMessage(item: unknown): string {
  if (typeof item !== 'object' || item === null) {
    return '';
  }

  const itemRecord = item as { message?: unknown };
  return toTrimmedString(itemRecord.message);
}

export function createBoardLegacyItemMetadataActionsUtils(): BoardLegacyItemMetadataActionsUtils {
  async function renameItem(params: RenameItemParams): Promise<void> {
    const flushed = await params.flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const currentTitle = toItemTitle(params.item);
    const currentMessage = toItemMessage(params.item);
    const nextMetadata = await params.openRenameDialog(currentTitle, currentMessage);

    if (nextMetadata === null) {
      return;
    }

    const itemId = toItemId(params.item);
    if (!itemId) {
      params.setStatusError('Renommage impossible: id introuvable.');
      return;
    }

    const nextTitle = toTrimmedString(nextMetadata.title);
    const nextMessage = toTrimmedString(nextMetadata.message);

    try {
      await params.patchItemMetadata(itemId, {
        title: nextTitle,
        message: nextMessage,
      });

      await params.loadItemsAndRender();
      const hasTitle = nextTitle.length > 0;
      const hasMessage = nextMessage.length > 0;
      params.setStatusSuccess(
        hasTitle || hasMessage
          ? `Mème mis à jour (${hasTitle ? 'nom' : 'sans nom'} / ${hasMessage ? 'message' : 'sans message'}).`
          : 'Nom et message supprimés.',
      );
    } catch (error) {
      params.setStatusError(`Renommage impossible: ${toErrorMessage(error)}`);
    }
  }

  async function deleteItem(params: DeleteItemParams): Promise<void> {
    const flushed = await params.flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const confirmed = await params.openDeleteDialog(params.item);
    if (!confirmed) {
      return;
    }

    const itemId = toItemId(params.item);
    if (!itemId) {
      params.setStatusError('Suppression impossible: id introuvable.');
      return;
    }

    try {
      await params.deleteItemById(itemId);
      params.clearSelectionIfMatches(itemId);

      const { nextBindings, removedCount } = params.removeBindingsForItem(itemId);
      let shortcutCleanupError: unknown = null;

      if (removedCount > 0) {
        try {
          await params.persistBindings(nextBindings);
        } catch (error) {
          shortcutCleanupError = error;
        }
      }

      await params.loadItemsAndRender();

      if (shortcutCleanupError) {
        params.setStatusError(
          `Mème supprimé, mais libération du raccourci impossible : ${toErrorMessage(shortcutCleanupError)}`,
        );
        return;
      }

      params.setStatusSuccess(removedCount > 0 ? 'Mème supprimé. Raccourci libéré.' : 'Mème supprimé.');
    } catch (error) {
      params.setStatusError(`Suppression impossible: ${toErrorMessage(error)}`);
    }
  }

  return {
    renameItem,
    deleteItem,
  };
}
