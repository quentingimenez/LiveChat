interface AddDialogNodes {
  addLinkButton: unknown;
  addOverlay: unknown;
  addForm: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
  addCancelButton: unknown;
}

interface RenameDialogNodes {
  renameOverlay: unknown;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
}

interface DeleteDialogNodes {
  deleteOverlay: unknown;
  deleteMessage: unknown;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
}

interface CaptureDialogNodes {
  captureOverlay: unknown;
  captureCurrentNode: unknown;
  captureCancelButton: unknown;
  captureSaveButton: unknown;
}

export interface BoardLegacyDialogUtils {
  isAddDialogReady(nodes: AddDialogNodes): boolean;
  isRenameDialogReady(nodes: RenameDialogNodes): boolean;
  isDeleteDialogReady(nodes: DeleteDialogNodes): boolean;
  isCaptureUiReady(nodes: CaptureDialogNodes): boolean;
  bindBackdropClose(overlayNode: unknown, closeDialog: unknown): void;
}

export function createBoardLegacyDialogUtils(): BoardLegacyDialogUtils {
  function isAddDialogReady(nodes: AddDialogNodes): boolean {
    return (
      nodes.addLinkButton instanceof HTMLElement &&
      nodes.addOverlay instanceof HTMLElement &&
      nodes.addForm instanceof HTMLFormElement &&
      nodes.addUrlInput instanceof HTMLInputElement &&
      nodes.addTitleInput instanceof HTMLInputElement &&
      nodes.addMessageInput instanceof HTMLTextAreaElement &&
      nodes.addRefreshInput instanceof HTMLInputElement &&
      nodes.addCancelButton instanceof HTMLElement
    );
  }

  function isRenameDialogReady(nodes: RenameDialogNodes): boolean {
    return (
      nodes.renameOverlay instanceof HTMLElement &&
      nodes.renameForm instanceof HTMLFormElement &&
      nodes.renameInput instanceof HTMLInputElement &&
      nodes.renameMessageInput instanceof HTMLTextAreaElement &&
      nodes.renameCancelButton instanceof HTMLElement
    );
  }

  function isDeleteDialogReady(nodes: DeleteDialogNodes): boolean {
    return (
      nodes.deleteOverlay instanceof HTMLElement &&
      nodes.deleteMessage instanceof HTMLElement &&
      nodes.deleteCancelButton instanceof HTMLElement &&
      nodes.deleteConfirmButton instanceof HTMLElement
    );
  }

  function isCaptureUiReady(nodes: CaptureDialogNodes): boolean {
    return (
      nodes.captureOverlay instanceof HTMLElement &&
      nodes.captureCurrentNode instanceof HTMLElement &&
      nodes.captureCancelButton instanceof HTMLElement &&
      nodes.captureSaveButton instanceof HTMLButtonElement
    );
  }

  function bindBackdropClose(overlayNode: unknown, closeDialog: unknown): void {
    if (!(overlayNode instanceof HTMLElement) || typeof closeDialog !== 'function') {
      return;
    }

    let pointerStartedOnBackdrop = false;

    overlayNode.addEventListener('mousedown', (event) => {
      pointerStartedOnBackdrop = event.target === overlayNode;
    });

    overlayNode.addEventListener('click', (event) => {
      const clickedBackdrop = event.target === overlayNode;

      if (clickedBackdrop && pointerStartedOnBackdrop) {
        closeDialog();
      }

      pointerStartedOnBackdrop = false;
    });
  }

  return {
    isAddDialogReady,
    isRenameDialogReady,
    isDeleteDialogReady,
    isCaptureUiReady,
    bindBackdropClose
  };
}
