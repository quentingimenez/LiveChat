interface KeyboardUpdatePayload {
  nextPendingAccelerator: string | null;
  nextPendingDisplay: string;
  statusMessage: string;
}

interface WireBoardInteractionsParams {
  bindKeyboardHandler: (params: unknown) => void;
  resolveDialogKeyboardAction: (key: string) => unknown;
  closeAddDialog: () => void;
  closeRenameDialog: () => void;
  closeDeleteDialogCancel: () => void;
  closeDeleteDialogConfirm: () => void;
  isCaptureActive: () => boolean;
  resolveCaptureKeyInput: (event: KeyboardEvent) => unknown;
  onCaptureCancelled: () => void;
  onCaptureUpdated: (payload: KeyboardUpdatePayload) => void;

  bindLifecycleHandlers: (params: unknown) => void;
  flushPreviewMessageOnLifecycleExit: () => void;
  schedulePreviewLayoutRefresh: () => void;

  bindCaptureUi: (params: unknown) => void;
  isCaptureUiReady: boolean;
  captureCancelButton: unknown;
  captureSaveButton: unknown;
  onCancelCapture: () => void;
  onSaveCapture: () => void;
  refreshCaptureUi: () => void;

  bindAddDialog: (params: unknown) => void;
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
  closeAddDialogWithValue: (value: unknown) => void;
  bindBackdropClose: (overlayNode: unknown, closeDialog: unknown) => void;

  bindAddLinkButton: (params: unknown) => void;
  addLinkButton: unknown;
  openAddDialog: () => Promise<unknown>;
  addItemFromLink: (payload: unknown) => Promise<void>;

  bindRenameDialog: (params: unknown) => void;
  isRenameDialogReady: boolean;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
  renameOverlay: unknown;
  closeRenameDialogWithValue: (value: unknown) => void;

  bindDeleteDialog: (params: unknown) => void;
  isDeleteDialogReady: boolean;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
  deleteOverlay: unknown;
  closeDeleteDialogWithValue: (value: unknown) => void;

  bindSearchControls: (params: unknown) => void;
  searchForm: unknown;
  searchInput: unknown;
  refreshButton: unknown;
  stopPlaybackButton: unknown;
  runSearchNow: () => Promise<void>;
  scheduleInstantSearch: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  loadItemsAndRender: () => Promise<void>;
  stopCurrentPlayback: () => Promise<void>;
}

export interface BoardLegacyWireupUtils {
  wireBoardInteractions(params: WireBoardInteractionsParams): void;
}

export function createBoardLegacyWireupUtils(): BoardLegacyWireupUtils {
  function wireBoardInteractions(params: WireBoardInteractionsParams): void {
    params.bindKeyboardHandler({
      resolveDialogKeyboardAction: params.resolveDialogKeyboardAction,
      closeAddDialog: params.closeAddDialog,
      closeRenameDialog: params.closeRenameDialog,
      closeDeleteDialogCancel: params.closeDeleteDialogCancel,
      closeDeleteDialogConfirm: params.closeDeleteDialogConfirm,
      isCaptureActive: params.isCaptureActive,
      resolveCaptureKeyInput: params.resolveCaptureKeyInput,
      onCaptureCancelled: params.onCaptureCancelled,
      onCaptureUpdated: params.onCaptureUpdated,
    });

    params.bindLifecycleHandlers({
      flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
    });

    params.bindCaptureUi({
      isCaptureUiReady: params.isCaptureUiReady,
      captureCancelButton: params.captureCancelButton,
      captureSaveButton: params.captureSaveButton,
      onCancelCapture: params.onCancelCapture,
      onSaveCapture: params.onSaveCapture,
      refreshCaptureUi: params.refreshCaptureUi,
    });

    params.bindAddDialog({
      isAddDialogReady: params.isAddDialogReady,
      addForm: params.addForm,
      addUrlInput: params.addUrlInput,
      addTitleInput: params.addTitleInput,
      addMessageInput: params.addMessageInput,
      addRefreshInput: params.addRefreshInput,
      addCancelButton: params.addCancelButton,
      addOverlay: params.addOverlay,
      isHttpUrl: params.isHttpUrl,
      setStatusError: params.setStatusError,
      closeAddDialog: params.closeAddDialogWithValue,
      bindBackdropClose: params.bindBackdropClose,
    });

    params.bindAddLinkButton({
      addLinkButton: params.addLinkButton,
      onOpenAndSubmit: () => {
        void (async () => {
          const payload = await params.openAddDialog();
          if (!payload) {
            return;
          }

          await params.addItemFromLink(payload);
        })();
      },
    });

    params.bindRenameDialog({
      isRenameDialogReady: params.isRenameDialogReady,
      renameForm: params.renameForm,
      renameInput: params.renameInput,
      renameMessageInput: params.renameMessageInput,
      renameCancelButton: params.renameCancelButton,
      renameOverlay: params.renameOverlay,
      closeRenameDialog: params.closeRenameDialogWithValue,
      bindBackdropClose: params.bindBackdropClose,
    });

    params.bindDeleteDialog({
      isDeleteDialogReady: params.isDeleteDialogReady,
      deleteCancelButton: params.deleteCancelButton,
      deleteConfirmButton: params.deleteConfirmButton,
      deleteOverlay: params.deleteOverlay,
      closeDeleteDialog: params.closeDeleteDialogWithValue,
      bindBackdropClose: params.bindBackdropClose,
    });

    params.bindSearchControls({
      searchForm: params.searchForm,
      searchInput: params.searchInput,
      refreshButton: params.refreshButton,
      stopPlaybackButton: params.stopPlaybackButton,
      onSearchSubmit: () => {
        void params.runSearchNow();
      },
      onSearchInput: () => {
        params.scheduleInstantSearch();
      },
      onRefresh: () => {
        void (async () => {
          const flushed = await params.flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }

          await params.loadItemsAndRender();
        })();
      },
      onStopPlayback: () => {
        void params.stopCurrentPlayback();
      },
    });
  }

  return {
    wireBoardInteractions,
  };
}
