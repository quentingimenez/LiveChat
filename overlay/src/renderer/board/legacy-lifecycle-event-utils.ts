interface DialogKeyboardResolution {
  shouldStopProcessing: boolean;
  shouldPreventDefault: boolean;
  action: string;
}

interface CaptureKeyResolution {
  kind: string;
  nextPendingAccelerator: string | null;
  nextPendingDisplay: string;
  statusMessage: string;
}

interface BindKeyboardHandlerParams {
  resolveDialogKeyboardAction: (key: string) => DialogKeyboardResolution;
  closeAddDialog: () => void;
  closeRenameDialog: () => void;
  closeDeleteDialogCancel: () => void;
  closeDeleteDialogConfirm: () => void;
  isCaptureActive: () => boolean;
  resolveCaptureKeyInput: (event: KeyboardEvent) => CaptureKeyResolution;
  onCaptureCancelled: () => void;
  onCaptureUpdated: (params: {
    nextPendingAccelerator: string | null;
    nextPendingDisplay: string;
    statusMessage: string;
  }) => void;
}

interface BindLifecycleHandlersParams {
  flushPreviewMessageOnLifecycleExit: () => void;
  schedulePreviewLayoutRefresh: () => void;
}

export interface BoardLegacyLifecycleEventUtils {
  bindKeyboardHandler(params: BindKeyboardHandlerParams): void;
  bindLifecycleHandlers(params: BindLifecycleHandlersParams): void;
}

function toDialogAction(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function toBoolean(value: unknown): boolean {
  return value === true;
}

function normalizeDialogKeyboardResolution(value: unknown): DialogKeyboardResolution {
  const record = typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
  return {
    shouldStopProcessing: toBoolean(record.shouldStopProcessing),
    shouldPreventDefault: toBoolean(record.shouldPreventDefault),
    action: toDialogAction(record.action),
  };
}

function normalizeCaptureKeyResolution(value: unknown): CaptureKeyResolution {
  const record = typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};

  return {
    kind: toDialogAction(record.kind),
    nextPendingAccelerator: typeof record.nextPendingAccelerator === 'string' ? record.nextPendingAccelerator : null,
    nextPendingDisplay: typeof record.nextPendingDisplay === 'string' ? record.nextPendingDisplay : '',
    statusMessage: typeof record.statusMessage === 'string' ? record.statusMessage : '',
  };
}

export function createBoardLegacyLifecycleEventUtils(): BoardLegacyLifecycleEventUtils {
  function bindKeyboardHandler(params: BindKeyboardHandlerParams): void {
    window.addEventListener('keydown', (event) => {
      const dialogKeyboardResolution = normalizeDialogKeyboardResolution(params.resolveDialogKeyboardAction(event.key));

      if (dialogKeyboardResolution.shouldStopProcessing) {
        if (dialogKeyboardResolution.shouldPreventDefault) {
          event.preventDefault();
        }

        if (dialogKeyboardResolution.action === 'close-add-dialog') {
          params.closeAddDialog();
        } else if (dialogKeyboardResolution.action === 'close-rename-dialog') {
          params.closeRenameDialog();
        } else if (dialogKeyboardResolution.action === 'close-delete-dialog-cancel') {
          params.closeDeleteDialogCancel();
        } else if (dialogKeyboardResolution.action === 'close-delete-dialog-confirm') {
          params.closeDeleteDialogConfirm();
        }

        return;
      }

      if (!params.isCaptureActive()) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();

      const resolution = normalizeCaptureKeyResolution(params.resolveCaptureKeyInput(event));
      if (resolution.kind === 'cancelled') {
        params.onCaptureCancelled();
        return;
      }

      params.onCaptureUpdated({
        nextPendingAccelerator: resolution.nextPendingAccelerator,
        nextPendingDisplay: resolution.nextPendingDisplay,
        statusMessage: resolution.statusMessage,
      });
    });
  }

  function bindLifecycleHandlers(params: BindLifecycleHandlersParams): void {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden') {
        params.flushPreviewMessageOnLifecycleExit();
      }
    });

    window.addEventListener('pagehide', () => {
      params.flushPreviewMessageOnLifecycleExit();
    });

    window.addEventListener('beforeunload', () => {
      params.flushPreviewMessageOnLifecycleExit();
    });

    window.addEventListener('resize', () => {
      params.schedulePreviewLayoutRefresh();
    });
  }

  return {
    bindKeyboardHandler,
    bindLifecycleHandlers,
  };
}
