import type { BoardLegacyDialogOrchestrationUtils } from './legacy-dialog-orchestration-utils';
import type { BoardLegacyDialogUtils } from './legacy-dialog-utils';
import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types';

interface CreateRuntimeDialogHandlersParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  isHttpUrl: (value: string) => boolean;
  toCardTitle: (item: unknown) => string;
  setStatusError: (message: string) => void;
}

type RuntimeDialogHandlers = ReturnType<BoardLegacyDialogOrchestrationUtils['createDialogHandlers']> & {
  bindBackdropClose: (overlayNode: unknown, closeDialog: unknown) => void;
};

export interface BoardLegacyRuntimeDialogHandlersUtils {
  createRuntimeDialogHandlers(params: CreateRuntimeDialogHandlersParams): RuntimeDialogHandlers;
}

function requireUtility<T>(utility: T | undefined, utilityName: string): T {
  if (utility === undefined || utility === null) {
    throw new Error(`Board legacy utility unavailable: ${utilityName}`);
  }

  return utility;
}

function createBindBackdropClose(dialogUtils: BoardLegacyDialogUtils): RuntimeDialogHandlers['bindBackdropClose'] {
  return (overlayNode, closeDialog) => {
    dialogUtils.bindBackdropClose(overlayNode, closeDialog);
  };
}

export function createBoardLegacyRuntimeDialogHandlersUtils(): BoardLegacyRuntimeDialogHandlersUtils {
  function createRuntimeDialogHandlers(params: CreateRuntimeDialogHandlersParams): RuntimeDialogHandlers {
    const dialogOrchestrationUtils = requireUtility<BoardLegacyDialogOrchestrationUtils>(
      window.__boardLegacyDialogOrchestrationUtils,
      'dialog-orchestration'
    );
    const dialogUtils = requireUtility(window.__boardLegacyDialogUtils, 'dialog');
    const dialogFlowUtils = requireUtility(window.__boardLegacyDialogFlowUtils, 'dialog-flow');
    const dialogStateUtils = requireUtility(window.__boardLegacyDialogStateUtils, 'dialog-state');

    const dialogHandlers = dialogOrchestrationUtils.createDialogHandlers({
      dialogUtils,
      dialogFlowUtils,
      dialogStateUtils,
      nodes: {
        addLinkButton: params.nodes.addLinkButton,
        addOverlay: params.nodes.addOverlay,
        addForm: params.nodes.addForm,
        addUrlInput: params.nodes.addUrlInput,
        addTitleInput: params.nodes.addTitleInput,
        addMessageInput: params.nodes.addMessageInput,
        addRefreshInput: params.nodes.addRefreshInput,
        addCancelButton: params.nodes.addCancelButton,
        renameOverlay: params.nodes.renameOverlay,
        renameForm: params.nodes.renameForm,
        renameInput: params.nodes.renameInput,
        renameMessageInput: params.nodes.renameMessageInput,
        renameCancelButton: params.nodes.renameCancelButton,
        deleteOverlay: params.nodes.deleteOverlay,
        deleteMessage: params.nodes.deleteMessage,
        deleteCancelButton: params.nodes.deleteCancelButton,
        deleteConfirmButton: params.nodes.deleteConfirmButton,
      },
      isHttpUrl: params.isHttpUrl,
      setStatusError: params.setStatusError,
      toCardTitle: params.toCardTitle,
      getResolveAddDialog: () => params.state.resolveAddDialog,
      setResolveAddDialog: (nextResolver) => {
        params.state.resolveAddDialog = nextResolver;
      },
      getResolveDeleteDialog: () => params.state.resolveDeleteDialog,
      setResolveDeleteDialog: (nextResolver) => {
        params.state.resolveDeleteDialog = nextResolver;
      },
      getResolveRenameDialog: () => params.state.resolveRenameDialog,
      setResolveRenameDialog: (nextResolver) => {
        params.state.resolveRenameDialog = nextResolver;
      },
    });

    return {
      ...dialogHandlers,
      bindBackdropClose: createBindBackdropClose(dialogUtils),
    };
  }

  return {
    createRuntimeDialogHandlers,
  };
}
