interface BoardWireupNodes {
  captureCancelButton: unknown;
  captureSaveButton: unknown;
  addForm: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
  addCancelButton: unknown;
  addOverlay: unknown;
  addLinkButton: unknown;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
  renameOverlay: unknown;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
  deleteOverlay: unknown;
  searchForm: unknown;
  searchInput: unknown;
  refreshButton: unknown;
  stopPlaybackButton: unknown;
}

interface BoardWireupState {
  resolveAddDialog: unknown;
  resolveRenameDialog: unknown;
  resolveDeleteDialog: unknown;
  captureItemId: unknown;
  capturePendingAccelerator: string | null;
  capturePendingDisplay: string;
}

interface RegisterBoardInteractionsParams {
  wireBoardInteractions: (params: unknown) => void;
  bindKeyboardHandler: (params: unknown) => void;
  resolveDialogKeyboardAction: (params: {
    key: string;
    hasAddDialog: boolean;
    hasRenameDialog: boolean;
    hasDeleteDialog: boolean;
  }) => unknown;
  resolveCaptureKeyInput: (params: {
    key: string;
    accelerator: string | null;
    display: string;
    currentPendingAccelerator: string | null;
    currentPendingDisplay: string;
    formatAcceleratorDisplay: (accelerator: string | null) => string;
  }) => unknown;
  bindLifecycleHandlers: (params: unknown) => void;
  flushPreviewMessageOnLifecycleExit: () => void;
  schedulePreviewLayoutRefresh: () => void;
  bindCaptureUi: (params: unknown) => void;
  bindAddDialog: (params: unknown) => void;
  bindAddLinkButton: (params: unknown) => void;
  bindRenameDialog: (params: unknown) => void;
  bindDeleteDialog: (params: unknown) => void;
  bindSearchControls: (params: unknown) => void;
  state: BoardWireupState;
  keyEventToAccelerator: (event: KeyboardEvent) => string | null;
  keyEventToDisplay: (event: KeyboardEvent) => string;
  formatAcceleratorDisplay: (accelerator: string | null) => string;
  endCapture: () => void;
  clearStatus: () => void;
  setStatusSuccess: (message: string) => void;
  closeAddDialog: (value: unknown) => void;
  closeRenameDialog: (value: unknown) => void;
  closeDeleteDialog: (value: unknown) => void;
  refreshCaptureUi: () => void;
  commitCapturedShortcut: () => Promise<void>;
  isCaptureUiReady: () => boolean;
  isAddDialogReady: () => boolean;
  isRenameDialogReady: () => boolean;
  isDeleteDialogReady: () => boolean;
  isHttpUrl: (value: string) => boolean;
  setStatusError: (message: string) => void;
  bindBackdropClose: (overlayNode: unknown, closeDialog: unknown) => void;
  openAddDialog: () => Promise<unknown>;
  addItemFromLink: (payload: unknown) => Promise<void>;
  runSearchNow: () => Promise<void>;
  scheduleInstantSearch: () => void;
  flushPreviewMessageChanges: () => Promise<boolean>;
  loadItemsAndRender: () => Promise<void>;
  stopCurrentPlayback: () => Promise<void>;
  nodes: BoardWireupNodes;
}

export interface BoardLegacyWireupRegistrationUtils {
  registerBoardInteractions(params: RegisterBoardInteractionsParams): void;
}

export function createBoardLegacyWireupRegistrationUtils(): BoardLegacyWireupRegistrationUtils {
  function registerBoardInteractions(params: RegisterBoardInteractionsParams): void {
    params.wireBoardInteractions({
      bindKeyboardHandler: params.bindKeyboardHandler,
      resolveDialogKeyboardAction: (key: string) =>
        params.resolveDialogKeyboardAction({
          key,
          hasAddDialog: !!params.state.resolveAddDialog,
          hasRenameDialog: !!params.state.resolveRenameDialog,
          hasDeleteDialog: !!params.state.resolveDeleteDialog,
        }),
      closeAddDialog: () => {
        params.closeAddDialog(null);
      },
      closeRenameDialog: () => {
        params.closeRenameDialog(null);
      },
      closeDeleteDialogCancel: () => {
        params.closeDeleteDialog(false);
      },
      closeDeleteDialogConfirm: () => {
        params.closeDeleteDialog(true);
      },
      isCaptureActive: () => !!params.state.captureItemId,
      resolveCaptureKeyInput: (event: KeyboardEvent) =>
        params.resolveCaptureKeyInput({
          key: event.key,
          accelerator: params.keyEventToAccelerator(event),
          display: params.keyEventToDisplay(event),
          currentPendingAccelerator: params.state.capturePendingAccelerator,
          currentPendingDisplay: params.state.capturePendingDisplay,
          formatAcceleratorDisplay: params.formatAcceleratorDisplay,
        }),
      onCaptureCancelled: () => {
        params.endCapture();
        params.clearStatus();
      },
      onCaptureUpdated: ({
        nextPendingAccelerator,
        nextPendingDisplay,
        statusMessage,
      }: {
        nextPendingAccelerator: string | null;
        nextPendingDisplay: string;
        statusMessage: string;
      }) => {
        params.state.capturePendingAccelerator = nextPendingAccelerator;
        params.state.capturePendingDisplay = nextPendingDisplay;
        params.refreshCaptureUi();
        params.setStatusSuccess(statusMessage);
      },
      bindLifecycleHandlers: params.bindLifecycleHandlers,
      flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
      bindCaptureUi: params.bindCaptureUi,
      isCaptureUiReady: params.isCaptureUiReady(),
      captureCancelButton: params.nodes.captureCancelButton,
      captureSaveButton: params.nodes.captureSaveButton,
      onCancelCapture: () => {
        params.endCapture();
        params.clearStatus();
      },
      onSaveCapture: () => {
        void params.commitCapturedShortcut();
      },
      refreshCaptureUi: params.refreshCaptureUi,
      bindAddDialog: params.bindAddDialog,
      isAddDialogReady: params.isAddDialogReady(),
      addForm: params.nodes.addForm,
      addUrlInput: params.nodes.addUrlInput,
      addTitleInput: params.nodes.addTitleInput,
      addMessageInput: params.nodes.addMessageInput,
      addRefreshInput: params.nodes.addRefreshInput,
      addCancelButton: params.nodes.addCancelButton,
      addOverlay: params.nodes.addOverlay,
      isHttpUrl: params.isHttpUrl,
      setStatusError: params.setStatusError,
      closeAddDialogWithValue: params.closeAddDialog,
      bindBackdropClose: params.bindBackdropClose,
      bindAddLinkButton: params.bindAddLinkButton,
      addLinkButton: params.nodes.addLinkButton,
      openAddDialog: params.openAddDialog,
      addItemFromLink: params.addItemFromLink,
      bindRenameDialog: params.bindRenameDialog,
      isRenameDialogReady: params.isRenameDialogReady(),
      renameForm: params.nodes.renameForm,
      renameInput: params.nodes.renameInput,
      renameMessageInput: params.nodes.renameMessageInput,
      renameCancelButton: params.nodes.renameCancelButton,
      renameOverlay: params.nodes.renameOverlay,
      closeRenameDialogWithValue: params.closeRenameDialog,
      bindDeleteDialog: params.bindDeleteDialog,
      isDeleteDialogReady: params.isDeleteDialogReady(),
      deleteCancelButton: params.nodes.deleteCancelButton,
      deleteConfirmButton: params.nodes.deleteConfirmButton,
      deleteOverlay: params.nodes.deleteOverlay,
      closeDeleteDialogWithValue: params.closeDeleteDialog,
      bindSearchControls: params.bindSearchControls,
      searchForm: params.nodes.searchForm,
      searchInput: params.nodes.searchInput,
      refreshButton: params.nodes.refreshButton,
      stopPlaybackButton: params.nodes.stopPlaybackButton,
      runSearchNow: params.runSearchNow,
      scheduleInstantSearch: params.scheduleInstantSearch,
      flushPreviewMessageChanges: params.flushPreviewMessageChanges,
      loadItemsAndRender: params.loadItemsAndRender,
      stopCurrentPlayback: params.stopCurrentPlayback,
    });
  }

  return {
    registerBoardInteractions,
  };
}
