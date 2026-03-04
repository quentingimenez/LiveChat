import type { BoardLegacyCaptureFlowUtils } from './legacy-capture-flow-utils.js';

interface CaptureState {
  captureItemId: string | null;
  capturePendingAccelerator: string | null;
  capturePendingDisplay: string;
}

interface CreateCaptureHandlersParams {
  captureFlowUtils: BoardLegacyCaptureFlowUtils;
  isCaptureUiReady: () => boolean;
  captureCurrentNode: unknown;
  captureSaveButton: unknown;
  captureOverlay: unknown;
  getCaptureState: () => CaptureState;
  setCaptureState: (nextState: CaptureState) => void;
  getBindings: () => unknown;
  assignShortcutForItem: (
    bindings: unknown,
    itemId: string,
    accelerator: string,
  ) => {
    nextBindings: Record<string, string>;
  };
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  renderList: () => void;
  formatAcceleratorDisplay: (accelerator: string) => string;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

export interface BoardCaptureHandlers {
  refreshCaptureUi(): void;
  endCapture(): void;
  commitCapturedShortcut(): Promise<void>;
  beginCaptureForItem(itemId: string): void;
}

export interface BoardLegacyCaptureOrchestrationUtils {
  createCaptureHandlers(params: CreateCaptureHandlersParams): BoardCaptureHandlers;
}

export function createBoardLegacyCaptureOrchestrationUtils(): BoardLegacyCaptureOrchestrationUtils {
  function createCaptureHandlers(params: CreateCaptureHandlersParams): BoardCaptureHandlers {
    function refreshCaptureUi(): void {
      params.captureFlowUtils.refreshCaptureUi({
        isCaptureUiReady: params.isCaptureUiReady(),
        captureCurrentNode: params.captureCurrentNode,
        captureSaveButton: params.captureSaveButton,
        state: params.getCaptureState(),
      });
    }

    function endCapture(): void {
      params.captureFlowUtils.endCapture({
        captureOverlay: params.captureOverlay,
        setCaptureState: params.setCaptureState,
        refreshCaptureUi,
      });
    }

    async function commitCapturedShortcut(): Promise<void> {
      await params.captureFlowUtils.commitCapturedShortcut({
        state: params.getCaptureState(),
        bindings: params.getBindings(),
        assignShortcutForItem: params.assignShortcutForItem,
        persistBindings: params.persistBindings,
        endCapture,
        renderList: params.renderList,
        formatAcceleratorDisplay: params.formatAcceleratorDisplay,
        setStatusSuccess: params.setStatusSuccess,
        setStatusError: (message) => {
          params.setStatusError(message || "Impossible d'assigner ce raccourci.");
        },
      });
    }

    function beginCaptureForItem(itemId: string): void {
      params.captureFlowUtils.beginCaptureForItem({
        itemId,
        captureOverlay: params.captureOverlay,
        setCaptureState: params.setCaptureState,
        refreshCaptureUi,
        setStatusSuccess: params.setStatusSuccess,
      });
    }

    return {
      refreshCaptureUi,
      endCapture,
      commitCapturedShortcut,
      beginCaptureForItem,
    };
  }

  return {
    createCaptureHandlers,
  };
}
