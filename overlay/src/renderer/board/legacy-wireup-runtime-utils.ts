import type { BoardLegacyCaptureUtils } from './legacy-capture-utils';
import type { BoardLegacyEventBindingsUtils } from './legacy-event-bindings-utils';
import type { BoardLegacyKeyboardUtils } from './legacy-keyboard-utils';
import type { BoardLegacyLifecycleEventUtils } from './legacy-lifecycle-event-utils';
import type { BoardLegacyWireupRegistrationUtils } from './legacy-wireup-registration-utils';
import type { BoardLegacyWireupUtils } from './legacy-wireup-utils';

interface BoardWireupState {
  resolveAddDialog: unknown;
  resolveRenameDialog: unknown;
  resolveDeleteDialog: unknown;
  captureItemId: unknown;
  capturePendingAccelerator: string | null;
  capturePendingDisplay: string;
}

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

interface RegisterWireupRuntimeParams {
  wireupRegistrationUtils: BoardLegacyWireupRegistrationUtils;
  wireupUtils: BoardLegacyWireupUtils;
  lifecycleEventUtils: BoardLegacyLifecycleEventUtils;
  keyboardUtils: BoardLegacyKeyboardUtils;
  captureUtils: BoardLegacyCaptureUtils;
  eventBindingsUtils: BoardLegacyEventBindingsUtils;
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
  flushPreviewMessageOnLifecycleExit: () => void;
  schedulePreviewLayoutRefresh: () => void;
  nodes: BoardWireupNodes;
}

export interface BoardLegacyWireupRuntimeUtils {
  registerWireupRuntime(params: RegisterWireupRuntimeParams): void;
}

export function createBoardLegacyWireupRuntimeUtils(): BoardLegacyWireupRuntimeUtils {
  function registerWireupRuntime(params: RegisterWireupRuntimeParams): void {
    params.wireupRegistrationUtils.registerBoardInteractions({
      wireBoardInteractions: (nextParams) =>
        params.wireupUtils.wireBoardInteractions(
          nextParams as Parameters<BoardLegacyWireupUtils['wireBoardInteractions']>[0],
        ),
      bindKeyboardHandler: (nextParams) =>
        params.lifecycleEventUtils.bindKeyboardHandler(
          nextParams as Parameters<BoardLegacyLifecycleEventUtils['bindKeyboardHandler']>[0],
        ),
      resolveDialogKeyboardAction: (nextParams) => params.keyboardUtils.resolveDialogKeyboardAction(nextParams),
      resolveCaptureKeyInput: (nextParams) => params.captureUtils.resolveCaptureKeyInput(nextParams),
      bindLifecycleHandlers: (nextParams) =>
        params.lifecycleEventUtils.bindLifecycleHandlers(
          nextParams as Parameters<BoardLegacyLifecycleEventUtils['bindLifecycleHandlers']>[0],
        ),
      flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
      bindCaptureUi: (nextParams) =>
        params.eventBindingsUtils.bindCaptureUi(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindCaptureUi']>[0],
        ),
      bindAddDialog: (nextParams) =>
        params.eventBindingsUtils.bindAddDialog(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindAddDialog']>[0],
        ),
      bindAddLinkButton: (nextParams) =>
        params.eventBindingsUtils.bindAddLinkButton(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindAddLinkButton']>[0],
        ),
      bindRenameDialog: (nextParams) =>
        params.eventBindingsUtils.bindRenameDialog(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindRenameDialog']>[0],
        ),
      bindDeleteDialog: (nextParams) =>
        params.eventBindingsUtils.bindDeleteDialog(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindDeleteDialog']>[0],
        ),
      bindSearchControls: (nextParams) =>
        params.eventBindingsUtils.bindSearchControls(
          nextParams as Parameters<BoardLegacyEventBindingsUtils['bindSearchControls']>[0],
        ),
      state: params.state,
      keyEventToAccelerator: params.keyEventToAccelerator,
      keyEventToDisplay: params.keyEventToDisplay,
      formatAcceleratorDisplay: params.formatAcceleratorDisplay,
      endCapture: params.endCapture,
      clearStatus: params.clearStatus,
      setStatusSuccess: params.setStatusSuccess,
      closeAddDialog: params.closeAddDialog,
      closeRenameDialog: params.closeRenameDialog,
      closeDeleteDialog: params.closeDeleteDialog,
      refreshCaptureUi: params.refreshCaptureUi,
      commitCapturedShortcut: params.commitCapturedShortcut,
      isCaptureUiReady: params.isCaptureUiReady,
      isAddDialogReady: params.isAddDialogReady,
      isRenameDialogReady: params.isRenameDialogReady,
      isDeleteDialogReady: params.isDeleteDialogReady,
      isHttpUrl: params.isHttpUrl,
      setStatusError: params.setStatusError,
      bindBackdropClose: params.bindBackdropClose,
      openAddDialog: params.openAddDialog,
      addItemFromLink: params.addItemFromLink,
      runSearchNow: params.runSearchNow,
      scheduleInstantSearch: params.scheduleInstantSearch,
      flushPreviewMessageChanges: params.flushPreviewMessageChanges,
      loadItemsAndRender: params.loadItemsAndRender,
      stopCurrentPlayback: params.stopCurrentPlayback,
      nodes: params.nodes,
    });
  }

  return {
    registerWireupRuntime,
  };
}
