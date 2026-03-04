interface PreviewMessagePatchPayload {
  title: string;
  message: string;
}

interface PreviewMessagePatchOptions {
  keepalive?: boolean;
}

interface SavedMessageContext {
  itemId: string;
  nextTitle: string;
  nextMessage: string;
}

export interface BoardLegacyPreviewMessageEditor {
  itemId: string;
  inputNode: unknown;
  lastSavedMessage: string;
  saving: boolean;
  inFlightSavePromise: Promise<boolean> | null;
}

export interface SavePreviewMessageParams {
  editor: BoardLegacyPreviewMessageEditor | null | undefined;
  waitForInFlight: boolean;
  keepalive: boolean;
  silent: boolean;
  resolveTitleForItem: (itemId: string) => string;
  patchItemMetadata: (
    itemId: string,
    payload: PreviewMessagePatchPayload,
    options: PreviewMessagePatchOptions,
  ) => Promise<void>;
  onSaved: (context: SavedMessageContext) => void;
  setStatusError: (message: string) => void;
  scheduleAutosave: (delayMs: number) => void;
}

export interface FlushPreviewMessageParams {
  editor: BoardLegacyPreviewMessageEditor | null | undefined;
  keepalive: boolean;
  silent: boolean;
  clearAutosave: () => void;
  resolveTitleForItem: (itemId: string) => string;
  patchItemMetadata: (
    itemId: string,
    payload: PreviewMessagePatchPayload,
    options: PreviewMessagePatchOptions,
  ) => Promise<void>;
  onSaved: (context: SavedMessageContext) => void;
  setStatusError: (message: string) => void;
  scheduleAutosave: (delayMs: number) => void;
}

export interface BoardLegacyPreviewMessageUtils {
  hasPendingPreviewMessageChanges(editor?: BoardLegacyPreviewMessageEditor | null): boolean;
  savePreviewMessage(params: SavePreviewMessageParams): Promise<boolean>;
  flushPreviewMessageChanges(params: FlushPreviewMessageParams): Promise<boolean>;
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

function isTextAreaNode(value: unknown): value is HTMLTextAreaElement {
  return typeof HTMLTextAreaElement === 'function' && value instanceof HTMLTextAreaElement;
}

export function createBoardLegacyPreviewMessageUtils(): BoardLegacyPreviewMessageUtils {
  function hasPendingPreviewMessageChanges(editor?: BoardLegacyPreviewMessageEditor | null): boolean {
    if (!editor || !isTextAreaNode(editor.inputNode)) {
      return false;
    }

    const draftValue = toTrimmedString(editor.inputNode.value);
    const savedValue = toTrimmedString(editor.lastSavedMessage);
    return draftValue !== savedValue;
  }

  async function savePreviewMessage(params: SavePreviewMessageParams): Promise<boolean> {
    const editor = params.editor;
    if (!editor || !isTextAreaNode(editor.inputNode)) {
      return false;
    }

    const itemId = toTrimmedString(editor.itemId);
    if (!itemId) {
      return false;
    }

    if (editor.saving) {
      if (params.waitForInFlight && editor.inFlightSavePromise) {
        try {
          await editor.inFlightSavePromise;
        } catch {
          // Ignore: the in-flight save already reported the failure.
        }
      } else {
        return !hasPendingPreviewMessageChanges(editor);
      }
    }

    const nextMessage = toTrimmedString(editor.inputNode.value);
    const previousSavedMessage = toTrimmedString(editor.lastSavedMessage);

    if (nextMessage === previousSavedMessage) {
      return true;
    }

    const nextTitle = toTrimmedString(params.resolveTitleForItem(itemId));

    editor.saving = true;
    const savePromise = (async () => {
      try {
        await params.patchItemMetadata(
          itemId,
          {
            title: nextTitle,
            message: nextMessage,
          },
          { keepalive: params.keepalive },
        );

        editor.lastSavedMessage = nextMessage;
        params.onSaved({
          itemId,
          nextTitle,
          nextMessage,
        });
        return true;
      } catch (error) {
        if (!params.silent) {
          params.setStatusError(`Enregistrement du message impossible: ${toErrorMessage(error)}`);
        }
        return false;
      }
    })();

    editor.inFlightSavePromise = savePromise;

    try {
      return await savePromise;
    } finally {
      if (editor.inFlightSavePromise === savePromise) {
        editor.inFlightSavePromise = null;
      }
      editor.saving = false;

      if (hasPendingPreviewMessageChanges(editor)) {
        params.scheduleAutosave(280);
      }
    }
  }

  async function flushPreviewMessageChanges(params: FlushPreviewMessageParams): Promise<boolean> {
    params.clearAutosave();

    const editor = params.editor;
    if (!editor || !isTextAreaNode(editor.inputNode)) {
      return true;
    }

    // Handle race: save in progress + user typed again while request was pending.
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const saved = await savePreviewMessage({
        editor,
        waitForInFlight: true,
        keepalive: params.keepalive,
        silent: params.silent,
        resolveTitleForItem: params.resolveTitleForItem,
        patchItemMetadata: params.patchItemMetadata,
        onSaved: params.onSaved,
        setStatusError: params.setStatusError,
        scheduleAutosave: params.scheduleAutosave,
      });

      if (!hasPendingPreviewMessageChanges(editor)) {
        return saved;
      }

      if (!saved) {
        return false;
      }
    }

    if (!params.silent) {
      params.setStatusError("Le message n'a pas pu être sauvegardé, action annulée.");
    }
    return false;
  }

  return {
    hasPendingPreviewMessageChanges,
    savePreviewMessage,
    flushPreviewMessageChanges,
  };
}
