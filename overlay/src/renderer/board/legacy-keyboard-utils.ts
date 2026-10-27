interface ResolveDialogKeyboardActionParams {
  key: unknown;
  hasAddDialog: boolean;
  hasRenameDialog: boolean;
  hasDeleteDialog: boolean;
}

type DialogKeyboardAction =
  | 'none'
  | 'close-add-dialog'
  | 'close-rename-dialog'
  | 'close-delete-dialog-cancel'
  | 'close-delete-dialog-confirm';

interface DialogKeyboardResolution {
  shouldStopProcessing: boolean;
  shouldPreventDefault: boolean;
  action: DialogKeyboardAction;
}

function toKeyString(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

export interface BoardLegacyKeyboardUtils {
  resolveDialogKeyboardAction(params: ResolveDialogKeyboardActionParams): DialogKeyboardResolution;
}

export function createBoardLegacyKeyboardUtils(): BoardLegacyKeyboardUtils {
  function resolveDialogKeyboardAction(params: ResolveDialogKeyboardActionParams): DialogKeyboardResolution {
    const key = toKeyString(params.key);

    if (params.hasAddDialog) {
      if (key === 'Escape') {
        return {
          shouldStopProcessing: true,
          shouldPreventDefault: true,
          action: 'close-add-dialog'
        };
      }

      return {
        shouldStopProcessing: true,
        shouldPreventDefault: false,
        action: 'none'
      };
    }

    if (params.hasRenameDialog) {
      if (key === 'Escape') {
        return {
          shouldStopProcessing: true,
          shouldPreventDefault: true,
          action: 'close-rename-dialog'
        };
      }

      return {
        shouldStopProcessing: true,
        shouldPreventDefault: false,
        action: 'none'
      };
    }

    if (params.hasDeleteDialog) {
      if (key === 'Escape') {
        return {
          shouldStopProcessing: true,
          shouldPreventDefault: true,
          action: 'close-delete-dialog-cancel'
        };
      }

      if (key === 'Enter') {
        return {
          shouldStopProcessing: true,
          shouldPreventDefault: true,
          action: 'close-delete-dialog-confirm'
        };
      }

      return {
        shouldStopProcessing: true,
        shouldPreventDefault: false,
        action: 'none'
      };
    }

    return {
      shouldStopProcessing: false,
      shouldPreventDefault: false,
      action: 'none'
    };
  }

  return {
    resolveDialogKeyboardAction
  };
}
