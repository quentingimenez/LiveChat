import type { BoardRuntimeNodes, BoardRuntimeState } from './legacy-runtime-types.js';

interface RegisterWireupParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  keyEventToAccelerator: (event: KeyboardEvent) => string | null;
  keyEventToDisplay: (event: KeyboardEvent) => string;
  formatAcceleratorDisplay: (accelerator: string | null) => string;
  endCapture: () => void;
  clearStatus: () => void;
  setStatusSuccess: (message: string) => void;
  setStatusError: (message: string) => void;
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
}

interface BootstrapRuntimeParams {
  state: BoardRuntimeState;
  nodes: BoardRuntimeNodes;
  getConfig: () => Promise<unknown>;
  getMemeBindings: () => Promise<unknown>;
  onSettings: (listener: (settings: unknown) => void) => void;
  applyPreviewVolume: () => void;
  setStatusError: (message: string) => void;
  schedulePreviewLayoutRefresh: () => void;
  refreshPreviewLayoutObserverTargets: () => void;
  loadItemsAndRender: () => Promise<void>;
}

interface RegisterAndBootstrapParams extends RegisterWireupParams, BootstrapRuntimeParams {}

export interface BoardLegacyRuntimeLifecycleUtils {
  registerWireupRuntime(params: RegisterWireupParams): void;
  bootstrapRuntime(params: BootstrapRuntimeParams): Promise<void>;
  registerWireupAndBootstrap(params: RegisterAndBootstrapParams): Promise<void>;
}

function requireUtility<T>(utility: T | undefined, utilityName: string): T {
  if (utility === undefined || utility === null) {
    throw new Error(`Board legacy utility unavailable: ${utilityName}`);
  }

  return utility;
}

export function createBoardLegacyRuntimeLifecycleUtils(): BoardLegacyRuntimeLifecycleUtils {
  function registerWireupRuntime(params: RegisterWireupParams): void {
    const wireupRuntimeUtils = requireUtility(window.__boardLegacyWireupRuntimeUtils, 'wireup-runtime');
    const wireupRegistrationUtils = requireUtility(window.__boardLegacyWireupRegistrationUtils, 'wireup-registration');
    const wireupUtils = requireUtility(window.__boardLegacyWireupUtils, 'wireup');
    const lifecycleEventUtils = requireUtility(window.__boardLegacyLifecycleEventUtils, 'lifecycle-events');
    const keyboardUtils = requireUtility(window.__boardLegacyKeyboardUtils, 'keyboard');
    const captureUtils = requireUtility(window.__boardLegacyCaptureUtils, 'capture');
    const eventBindingsUtils = requireUtility(window.__boardLegacyEventBindingsUtils, 'event-bindings');

    wireupRuntimeUtils.registerWireupRuntime({
      wireupRegistrationUtils,
      wireupUtils,
      lifecycleEventUtils,
      keyboardUtils,
      captureUtils,
      eventBindingsUtils,
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
      flushPreviewMessageOnLifecycleExit: params.flushPreviewMessageOnLifecycleExit,
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
      nodes: {
        captureCancelButton: params.nodes.captureCancelButton,
        captureSaveButton: params.nodes.captureSaveButton,
        addForm: params.nodes.addForm,
        addUrlInput: params.nodes.addUrlInput,
        addTitleInput: params.nodes.addTitleInput,
        addMessageInput: params.nodes.addMessageInput,
        addRefreshInput: params.nodes.addRefreshInput,
        addCancelButton: params.nodes.addCancelButton,
        addOverlay: params.nodes.addOverlay,
        addLinkButton: params.nodes.addLinkButton,
        renameForm: params.nodes.renameForm,
        renameInput: params.nodes.renameInput,
        renameMessageInput: params.nodes.renameMessageInput,
        renameCancelButton: params.nodes.renameCancelButton,
        renameOverlay: params.nodes.renameOverlay,
        deleteCancelButton: params.nodes.deleteCancelButton,
        deleteConfirmButton: params.nodes.deleteConfirmButton,
        deleteOverlay: params.nodes.deleteOverlay,
        searchForm: params.nodes.searchForm,
        searchInput: params.nodes.searchInput,
        refreshButton: params.nodes.refreshButton,
        stopPlaybackButton: params.nodes.stopPlaybackButton,
      },
    });
  }

  async function bootstrapRuntime(params: BootstrapRuntimeParams): Promise<void> {
    const bootstrapOrchestrationUtils = requireUtility(
      window.__boardLegacyBootstrapOrchestrationUtils,
      'bootstrap-orchestration'
    );
    const bootstrapUtils = requireUtility(window.__boardLegacyBootstrapUtils, 'bootstrap');

    await bootstrapOrchestrationUtils.bootstrapBoardRuntime({
      bootstrapBoard: (nextParams) =>
        bootstrapUtils.bootstrapBoard(nextParams as Parameters<(typeof bootstrapUtils)['bootstrapBoard']>[0]),
      getConfig: params.getConfig,
      getMemeBindings: params.getMemeBindings,
      onSettings: params.onSettings,
      state: params.state as {
        config: Record<string, unknown> | null;
        bindings: Record<string, string> | null;
        previewLayoutObserver: ResizeObserver | null;
      },
      applyPreviewVolume: params.applyPreviewVolume,
      setStatusError: params.setStatusError,
      refreshButton: params.nodes.refreshButton,
      searchInput: params.nodes.searchInput,
      stopPlaybackButton: params.nodes.stopPlaybackButton,
      addLinkButton: params.nodes.addLinkButton,
      previewStageNode: params.nodes.previewStageNode,
      schedulePreviewLayoutRefresh: params.schedulePreviewLayoutRefresh,
      refreshPreviewLayoutObserverTargets: params.refreshPreviewLayoutObserverTargets,
      loadItemsAndRender: params.loadItemsAndRender,
    });
  }

  async function registerWireupAndBootstrap(params: RegisterAndBootstrapParams): Promise<void> {
    registerWireupRuntime(params);
    await bootstrapRuntime(params);
  }

  return {
    registerWireupRuntime,
    bootstrapRuntime,
    registerWireupAndBootstrap,
  };
}
