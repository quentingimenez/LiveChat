interface BindCaptureUiParams {
  isCaptureUiReady: boolean;
  captureCancelButton: unknown;
  captureSaveButton: unknown;
  onCancelCapture: () => void;
  onSaveCapture: () => void;
  refreshCaptureUi: () => void;
}

interface BindAddDialogParams {
  isAddDialogReady: boolean;
  addForm: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
  addCancelButton: unknown;
  addOverlay: unknown;
  isHttpUrl: (value: string) => boolean;
  setStatusError: (message: string) => void;
  closeAddDialog: (value: unknown) => void;
  bindBackdropClose: (overlayNode: unknown, onClose: () => void) => void;
}

interface BindAddLinkButtonParams {
  addLinkButton: unknown;
  onOpenAndSubmit: () => void;
}

interface BindRenameDialogParams {
  isRenameDialogReady: boolean;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
  renameOverlay: unknown;
  closeRenameDialog: (value: unknown) => void;
  bindBackdropClose: (overlayNode: unknown, onClose: () => void) => void;
}

interface BindDeleteDialogParams {
  isDeleteDialogReady: boolean;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
  deleteOverlay: unknown;
  closeDeleteDialog: (value: boolean) => void;
  bindBackdropClose: (overlayNode: unknown, onClose: () => void) => void;
}

interface BindSearchControlsParams {
  searchForm: unknown;
  searchInput: unknown;
  refreshButton: unknown;
  stopPlaybackButton: unknown;
  onSearchSubmit: () => void;
  onSearchInput: () => void;
  onRefresh: () => void;
  onStopPlayback: () => void;
}

export interface BoardLegacyEventBindingsUtils {
  bindCaptureUi(params: BindCaptureUiParams): void;
  bindAddDialog(params: BindAddDialogParams): void;
  bindAddLinkButton(params: BindAddLinkButtonParams): void;
  bindRenameDialog(params: BindRenameDialogParams): void;
  bindDeleteDialog(params: BindDeleteDialogParams): void;
  bindSearchControls(params: BindSearchControlsParams): void;
}

export function createBoardLegacyEventBindingsUtils(): BoardLegacyEventBindingsUtils {
  function bindCaptureUi(params: BindCaptureUiParams): void {
    if (!params.isCaptureUiReady) {
      return;
    }

    if (params.captureCancelButton instanceof HTMLElement) {
      params.captureCancelButton.addEventListener('click', () => {
        params.onCancelCapture();
      });
    }

    if (params.captureSaveButton instanceof HTMLElement) {
      params.captureSaveButton.addEventListener('click', () => {
        params.onSaveCapture();
      });
    }

    params.refreshCaptureUi();
  }

  function bindAddDialog(params: BindAddDialogParams): void {
    if (!params.isAddDialogReady) {
      return;
    }

    if (
      params.addForm instanceof HTMLFormElement &&
      params.addUrlInput instanceof HTMLInputElement &&
      params.addTitleInput instanceof HTMLInputElement &&
      params.addMessageInput instanceof HTMLTextAreaElement &&
      params.addRefreshInput instanceof HTMLInputElement
    ) {
      const addUrlInput = params.addUrlInput;
      const addTitleInput = params.addTitleInput;
      const addMessageInput = params.addMessageInput;
      const addRefreshInput = params.addRefreshInput;

      params.addForm.addEventListener('submit', (event) => {
        event.preventDefault();

        const url = `${addUrlInput.value || ''}`.trim();
        const title = `${addTitleInput.value || ''}`.trim();
        const message = `${addMessageInput.value || ''}`.trim();
        const forceRefresh = !!addRefreshInput.checked;

        if (!params.isHttpUrl(url)) {
          params.setStatusError('Lien invalide. Utilise une URL complète en http(s).');
          addUrlInput.focus();
          return;
        }

        params.closeAddDialog({
          url,
          title,
          message,
          forceRefresh
        });
      });
    }

    if (params.addCancelButton instanceof HTMLElement) {
      params.addCancelButton.addEventListener('click', () => {
        params.closeAddDialog(null);
      });
    }

    params.bindBackdropClose(params.addOverlay, () => {
      params.closeAddDialog(null);
    });
  }

  function bindAddLinkButton(params: BindAddLinkButtonParams): void {
    if (!(params.addLinkButton instanceof HTMLElement)) {
      return;
    }

    params.addLinkButton.addEventListener('click', () => {
      params.onOpenAndSubmit();
    });
  }

  function bindRenameDialog(params: BindRenameDialogParams): void {
    if (!params.isRenameDialogReady) {
      return;
    }

    if (
      params.renameForm instanceof HTMLFormElement &&
      params.renameInput instanceof HTMLInputElement &&
      params.renameMessageInput instanceof HTMLTextAreaElement
    ) {
      const renameInput = params.renameInput;
      const renameMessageInput = params.renameMessageInput;

      params.renameForm.addEventListener('submit', (event) => {
        event.preventDefault();
        params.closeRenameDialog({
          title: renameInput.value,
          message: renameMessageInput.value
        });
      });
    }

    if (params.renameCancelButton instanceof HTMLElement) {
      params.renameCancelButton.addEventListener('click', () => {
        params.closeRenameDialog(null);
      });
    }

    params.bindBackdropClose(params.renameOverlay, () => {
      params.closeRenameDialog(null);
    });
  }

  function bindDeleteDialog(params: BindDeleteDialogParams): void {
    if (!params.isDeleteDialogReady) {
      return;
    }

    if (params.deleteCancelButton instanceof HTMLElement) {
      params.deleteCancelButton.addEventListener('click', () => {
        params.closeDeleteDialog(false);
      });
    }

    if (params.deleteConfirmButton instanceof HTMLElement) {
      params.deleteConfirmButton.addEventListener('click', () => {
        params.closeDeleteDialog(true);
      });
    }

    params.bindBackdropClose(params.deleteOverlay, () => {
      params.closeDeleteDialog(false);
    });
  }

  function bindSearchControls(params: BindSearchControlsParams): void {
    if (params.searchForm instanceof HTMLFormElement) {
      params.searchForm.addEventListener('submit', (event) => {
        event.preventDefault();
        params.onSearchSubmit();
      });
    }

    if (params.searchInput instanceof HTMLInputElement) {
      params.searchInput.addEventListener('input', () => {
        params.onSearchInput();
      });
    }

    if (params.refreshButton instanceof HTMLElement) {
      params.refreshButton.addEventListener('click', () => {
        params.onRefresh();
      });
    }

    if (params.stopPlaybackButton instanceof HTMLElement) {
      params.stopPlaybackButton.addEventListener('click', () => {
        params.onStopPlayback();
      });
    }
  }

  return {
    bindCaptureUi,
    bindAddDialog,
    bindAddLinkButton,
    bindRenameDialog,
    bindDeleteDialog,
    bindSearchControls
  };
}
