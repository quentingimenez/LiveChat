import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types.js';

interface CreateRuntimeCaptureHandlersParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  isCaptureUiReady: () => boolean;
  persistBindings: (nextBindings: Record<string, string>) => Promise<void>;
  renderList: () => void;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
}

interface RuntimeCaptureHandlers {
  refreshCaptureUi: () => void;
  endCapture: () => void;
  commitCapturedShortcut: () => Promise<void>;
  beginCaptureForItem: (itemId: string) => void;
  formatAcceleratorDisplay: (accelerator: string | null) => string;
  keyEventToDisplay: (event: KeyboardEvent) => string;
  keyEventToAccelerator: (event: KeyboardEvent) => string | null;
}

export interface BoardLegacyRuntimeCaptureHandlersUtils {
  createRuntimeCaptureHandlers(params: CreateRuntimeCaptureHandlersParams): RuntimeCaptureHandlers;
}

function requireUtility<T>(utility: T | undefined, utilityName: string): T {
  if (utility === undefined || utility === null) {
    throw new Error(`Board legacy utility unavailable: ${utilityName}`);
  }

  return utility;
}

export function createBoardLegacyRuntimeCaptureHandlersUtils(): BoardLegacyRuntimeCaptureHandlersUtils {
  function createRuntimeCaptureHandlers(params: CreateRuntimeCaptureHandlersParams): RuntimeCaptureHandlers {
    const captureFlowUtils = requireUtility(window.__boardLegacyCaptureFlowUtils, 'capture-flow');
    const captureOrchestrationUtils = requireUtility(window.__boardLegacyCaptureOrchestrationUtils, 'capture-orchestration');
    const bindingsUtils = requireUtility(window.__boardLegacyBindingsUtils, 'bindings');
    const shortcutUtils = requireUtility(window.__boardLegacyShortcutUtils, 'shortcut');

    const formatAcceleratorDisplay = (accelerator: string | null): string =>
      shortcutUtils.formatAcceleratorDisplay(accelerator);

    const captureHandlers = captureOrchestrationUtils.createCaptureHandlers({
      captureFlowUtils,
      isCaptureUiReady: params.isCaptureUiReady,
      captureCurrentNode: params.nodes.captureCurrentNode,
      captureSaveButton: params.nodes.captureSaveButton,
      captureOverlay: params.nodes.captureOverlay,
      getCaptureState: () => ({
        captureItemId: params.state.captureItemId,
        capturePendingAccelerator: params.state.capturePendingAccelerator,
        capturePendingDisplay: params.state.capturePendingDisplay,
      }),
      setCaptureState: ({ captureItemId, capturePendingAccelerator, capturePendingDisplay }) => {
        params.state.captureItemId = captureItemId;
        params.state.capturePendingAccelerator = capturePendingAccelerator;
        params.state.capturePendingDisplay = capturePendingDisplay;
      },
      getBindings: () => params.state.bindings,
      assignShortcutForItem: bindingsUtils.assignShortcutForItem,
      persistBindings: params.persistBindings,
      renderList: params.renderList,
      formatAcceleratorDisplay,
      setStatusSuccess: params.setStatusSuccess,
      setStatusError: params.setStatusError,
    });

    return {
      ...captureHandlers,
      formatAcceleratorDisplay,
      keyEventToDisplay: shortcutUtils.keyEventToDisplay,
      keyEventToAccelerator: shortcutUtils.keyEventToAccelerator,
    };
  }

  return {
    createRuntimeCaptureHandlers,
  };
}
