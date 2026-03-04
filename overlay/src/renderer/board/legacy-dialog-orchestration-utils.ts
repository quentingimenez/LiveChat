import type { AddDialogPayload, BoardLegacyDialogFlowUtils, RenameDialogPayload } from './legacy-dialog-flow-utils.js';
import type { BoardLegacyDialogStateUtils } from './legacy-dialog-state-utils.js';
import type { BoardLegacyDialogUtils } from './legacy-dialog-utils.js';

interface BoardDialogNodes {
  addLinkButton: unknown;
  addOverlay: unknown;
  addForm: unknown;
  addUrlInput: unknown;
  addTitleInput: unknown;
  addMessageInput: unknown;
  addRefreshInput: unknown;
  addCancelButton: unknown;
  renameOverlay: unknown;
  renameForm: unknown;
  renameInput: unknown;
  renameMessageInput: unknown;
  renameCancelButton: unknown;
  deleteOverlay: unknown;
  deleteMessage: unknown;
  deleteCancelButton: unknown;
  deleteConfirmButton: unknown;
}

interface CreateDialogHandlersParams {
  dialogUtils: BoardLegacyDialogUtils;
  dialogFlowUtils: BoardLegacyDialogFlowUtils;
  dialogStateUtils: BoardLegacyDialogStateUtils;
  nodes: BoardDialogNodes;
  isHttpUrl: (value: string) => boolean;
  setStatusError: (message: string) => void;
  toCardTitle: (item: unknown) => string;
  getResolveAddDialog: () => unknown;
  setResolveAddDialog: (nextResolver: ((value: AddDialogPayload | null) => void) | null) => void;
  getResolveDeleteDialog: () => unknown;
  setResolveDeleteDialog: (nextResolver: ((value: boolean) => void) | null) => void;
  getResolveRenameDialog: () => unknown;
  setResolveRenameDialog: (nextResolver: ((value: RenameDialogPayload | null) => void) | null) => void;
}

interface BoardDialogHandlers {
  isAddDialogReady(): boolean;
  closeAddDialog(value: AddDialogPayload | null): void;
  openAddDialog(): Promise<AddDialogPayload | null>;
  isDeleteDialogReady(): boolean;
  closeDeleteDialog(value: boolean): void;
  openDeleteDialog(item: unknown): Promise<boolean>;
  isRenameDialogReady(): boolean;
  closeRenameDialog(value: RenameDialogPayload | null): void;
  openRenameDialog(title: string, message: string): Promise<RenameDialogPayload | null>;
}

export interface BoardLegacyDialogOrchestrationUtils {
  createDialogHandlers(params: CreateDialogHandlersParams): BoardDialogHandlers;
}

export function createBoardLegacyDialogOrchestrationUtils(): BoardLegacyDialogOrchestrationUtils {
  function createDialogHandlers(params: CreateDialogHandlersParams): BoardDialogHandlers {
    function isAddDialogReady(): boolean {
      return params.dialogUtils.isAddDialogReady({
        addLinkButton: params.nodes.addLinkButton,
        addOverlay: params.nodes.addOverlay,
        addForm: params.nodes.addForm,
        addUrlInput: params.nodes.addUrlInput,
        addTitleInput: params.nodes.addTitleInput,
        addMessageInput: params.nodes.addMessageInput,
        addRefreshInput: params.nodes.addRefreshInput,
        addCancelButton: params.nodes.addCancelButton,
      });
    }

    function closeAddDialog(value: AddDialogPayload | null): void {
      params.dialogStateUtils.closeDialogWithResolver({
        overlayNode: params.nodes.addOverlay,
        value,
        getResolver: params.getResolveAddDialog,
        setResolver: (nextResolver) => {
          params.setResolveAddDialog(nextResolver as ((nextValue: AddDialogPayload | null) => void) | null);
        },
      });
    }

    function openAddDialog(): Promise<AddDialogPayload | null> {
      return params.dialogFlowUtils.openAddDialog({
        isReady: isAddDialogReady(),
        nodes: {
          addOverlay: params.nodes.addOverlay,
          addUrlInput: params.nodes.addUrlInput,
          addTitleInput: params.nodes.addTitleInput,
          addMessageInput: params.nodes.addMessageInput,
          addRefreshInput: params.nodes.addRefreshInput,
        },
        isHttpUrl: params.isHttpUrl,
        setStatusError: params.setStatusError,
        closeDialog: closeAddDialog,
        createResolverPromise: () =>
          params.dialogStateUtils.createResolverPromise((nextResolver) => {
            params.setResolveAddDialog(nextResolver);
          }),
      });
    }

    function isDeleteDialogReady(): boolean {
      return params.dialogUtils.isDeleteDialogReady({
        deleteOverlay: params.nodes.deleteOverlay,
        deleteMessage: params.nodes.deleteMessage,
        deleteCancelButton: params.nodes.deleteCancelButton,
        deleteConfirmButton: params.nodes.deleteConfirmButton,
      });
    }

    function closeDeleteDialog(value: boolean): void {
      params.dialogStateUtils.closeDialogWithResolver({
        overlayNode: params.nodes.deleteOverlay,
        value,
        getResolver: params.getResolveDeleteDialog,
        setResolver: (nextResolver) => {
          params.setResolveDeleteDialog(nextResolver as ((nextValue: boolean) => void) | null);
        },
      });
    }

    function openDeleteDialog(item: unknown): Promise<boolean> {
      return params.dialogFlowUtils.openDeleteDialog({
        title: params.toCardTitle(item),
        isReady: isDeleteDialogReady(),
        nodes: {
          deleteMessage: params.nodes.deleteMessage,
          deleteOverlay: params.nodes.deleteOverlay,
          deleteConfirmButton: params.nodes.deleteConfirmButton,
        },
        closeDialog: closeDeleteDialog,
        createResolverPromise: () =>
          params.dialogStateUtils.createResolverPromise((nextResolver) => {
            params.setResolveDeleteDialog(nextResolver);
          }),
      });
    }

    function isRenameDialogReady(): boolean {
      return params.dialogUtils.isRenameDialogReady({
        renameOverlay: params.nodes.renameOverlay,
        renameForm: params.nodes.renameForm,
        renameInput: params.nodes.renameInput,
        renameMessageInput: params.nodes.renameMessageInput,
        renameCancelButton: params.nodes.renameCancelButton,
      });
    }

    function closeRenameDialog(value: RenameDialogPayload | null): void {
      params.dialogStateUtils.closeDialogWithResolver({
        overlayNode: params.nodes.renameOverlay,
        value,
        getResolver: params.getResolveRenameDialog,
        setResolver: (nextResolver) => {
          params.setResolveRenameDialog(nextResolver as ((nextValue: RenameDialogPayload | null) => void) | null);
        },
      });
    }

    function openRenameDialog(title: string, message: string): Promise<RenameDialogPayload | null> {
      return params.dialogFlowUtils.openRenameDialog({
        title,
        message,
        isReady: isRenameDialogReady(),
        nodes: {
          renameInput: params.nodes.renameInput,
          renameMessageInput: params.nodes.renameMessageInput,
          renameOverlay: params.nodes.renameOverlay,
        },
        closeDialog: closeRenameDialog,
        createResolverPromise: () =>
          params.dialogStateUtils.createResolverPromise((nextResolver) => {
            params.setResolveRenameDialog(nextResolver);
          }),
      });
    }

    return {
      isAddDialogReady,
      closeAddDialog,
      openAddDialog,
      isDeleteDialogReady,
      closeDeleteDialog,
      openDeleteDialog,
      isRenameDialogReady,
      closeRenameDialog,
      openRenameDialog,
    };
  }

  return {
    createDialogHandlers,
  };
}
