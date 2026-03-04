import type { BoardLegacyPreviewMessageEditor, BoardLegacyPreviewMessageUtils } from './legacy-preview-message-utils';

interface PreviewMessageSaveOptions {
  editor?: BoardLegacyPreviewMessageEditor | null;
  waitForInFlight?: boolean;
  keepalive?: boolean;
  silent?: boolean;
}

interface PreviewMessageFlushOptions {
  editor?: BoardLegacyPreviewMessageEditor | null;
  keepalive?: boolean;
  silent?: boolean;
}

interface CreatePreviewMessageHandlersParams {
  previewMessageUtils: BoardLegacyPreviewMessageUtils;
  defaultAutosaveDelayMs: number;
  getPreviewMessageEditor: () => BoardLegacyPreviewMessageEditor | null;
  getPreviewMessageAutosaveTimeoutId: () => number | null;
  setPreviewMessageAutosaveTimeoutId: (nextTimeoutId: number | null) => void;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearTimeout: (timeoutId: number) => void;
  resolvePreviewMessageItemTitle: (itemId: string) => string;
  patchItemMetadata: (itemId: string, payload: { title: string; message: string }, options: { keepalive?: boolean }) => Promise<void>;
  applyLocalItemMetadata: (itemId: string, nextTitle: string, nextMessage: string) => void;
  renderList: () => void;
  setStatusError: (message: string) => void;
}

export interface BoardPreviewMessageHandlers {
  clearPreviewMessageAutosave(): void;
  hasPendingPreviewMessageChanges(editor?: BoardLegacyPreviewMessageEditor | null): boolean;
  schedulePreviewMessageAutosave(delayMs?: number): void;
  savePreviewMessage(options?: PreviewMessageSaveOptions): Promise<boolean>;
  flushPreviewMessageChanges(options?: PreviewMessageFlushOptions): Promise<boolean>;
  flushPreviewMessageOnLifecycleExit(): void;
}

export interface BoardLegacyPreviewMessageOrchestrationUtils {
  createPreviewMessageHandlers(params: CreatePreviewMessageHandlersParams): BoardPreviewMessageHandlers;
}

interface PersistenceOptions {
  keepalive?: boolean;
  silent?: boolean;
}

export function createBoardLegacyPreviewMessageOrchestrationUtils(): BoardLegacyPreviewMessageOrchestrationUtils {
  function createPreviewMessageHandlers(params: CreatePreviewMessageHandlersParams): BoardPreviewMessageHandlers {
    function clearPreviewMessageAutosave(): void {
      const timeoutId = params.getPreviewMessageAutosaveTimeoutId();
      if (timeoutId !== null) {
        params.clearTimeout(timeoutId);
        params.setPreviewMessageAutosaveTimeoutId(null);
      }
    }

    function hasPendingPreviewMessageChanges(editor = params.getPreviewMessageEditor()): boolean {
      return params.previewMessageUtils.hasPendingPreviewMessageChanges(editor);
    }

    function buildPersistenceParams(editor: BoardLegacyPreviewMessageEditor | null, options: PersistenceOptions = {}) {
      return {
        editor,
        keepalive: options.keepalive === true,
        silent: options.silent === true,
        resolveTitleForItem: params.resolvePreviewMessageItemTitle,
        patchItemMetadata: params.patchItemMetadata,
        onSaved: ({ itemId, nextTitle, nextMessage }: { itemId: string; nextTitle: string; nextMessage: string }) => {
          params.applyLocalItemMetadata(itemId, nextTitle, nextMessage);
          params.renderList();
        },
        setStatusError: params.setStatusError,
        scheduleAutosave: (delayMs: number) => {
          schedulePreviewMessageAutosave(delayMs);
        },
      };
    }

    function schedulePreviewMessageAutosave(delayMs = params.defaultAutosaveDelayMs): void {
      clearPreviewMessageAutosave();

      if (!params.getPreviewMessageEditor()) {
        return;
      }

      const timeoutId = params.setTimeout(() => {
        params.setPreviewMessageAutosaveTimeoutId(null);
        void savePreviewMessage();
      }, Math.max(0, Math.floor(delayMs)));

      params.setPreviewMessageAutosaveTimeoutId(timeoutId);
    }

    function savePreviewMessage(options: PreviewMessageSaveOptions = {}): Promise<boolean> {
      const editor = options.editor || params.getPreviewMessageEditor();
      return params.previewMessageUtils.savePreviewMessage({
        ...buildPersistenceParams(editor, options),
        waitForInFlight: options.waitForInFlight !== false,
      });
    }

    function flushPreviewMessageChanges(options: PreviewMessageFlushOptions = {}): Promise<boolean> {
      const editor = options.editor || params.getPreviewMessageEditor();
      return params.previewMessageUtils.flushPreviewMessageChanges({
        ...buildPersistenceParams(editor, options),
        clearAutosave: clearPreviewMessageAutosave,
      });
    }

    function flushPreviewMessageOnLifecycleExit(): void {
      const editor = params.getPreviewMessageEditor();
      if (!hasPendingPreviewMessageChanges(editor)) {
        return;
      }

      void flushPreviewMessageChanges({
        editor,
        keepalive: true,
        silent: true,
      });
    }

    return {
      clearPreviewMessageAutosave,
      hasPendingPreviewMessageChanges,
      schedulePreviewMessageAutosave,
      savePreviewMessage,
      flushPreviewMessageChanges,
      flushPreviewMessageOnLifecycleExit,
    };
  }

  return {
    createPreviewMessageHandlers,
  };
}
