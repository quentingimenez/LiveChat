interface BoardBootstrapState {
  config: Record<string, unknown> | null;
  bindings: Record<string, string> | null;
  previewLayoutObserver: ResizeObserver | null;
}

interface BootstrapBoardParams {
  bootstrapBoard: (params: unknown) => Promise<void>;
  getConfig: () => Promise<unknown>;
  getMemeBindings: () => Promise<unknown>;
  onSettings: (listener: (settings: unknown) => void) => void;
  state: BoardBootstrapState;
  applyPreviewVolume: () => void;
  setStatusError: (message: string) => void;
  refreshButton: unknown;
  searchInput: unknown;
  stopPlaybackButton: unknown;
  addLinkButton: unknown;
  previewStageNode: unknown;
  schedulePreviewLayoutRefresh: () => void;
  refreshPreviewLayoutObserverTargets: () => void;
  loadItemsAndRender: () => Promise<void>;
}

export interface BoardLegacyBootstrapOrchestrationUtils {
  bootstrapBoardRuntime(params: BootstrapBoardParams): Promise<void>;
}

export function createBoardLegacyBootstrapOrchestrationUtils(): BoardLegacyBootstrapOrchestrationUtils {
  async function bootstrapBoardRuntime(params: BootstrapBoardParams): Promise<void> {
    await params.bootstrapBoard({
      getConfig: params.getConfig,
      getMemeBindings: params.getMemeBindings,
      onSettings: params.onSettings,
      setConfig: (config: Record<string, unknown>) => {
        params.state.config = config;
      },
      mergeConfig: (nextSettings: unknown) => {
        params.state.config = {
          ...(params.state.config || {}),
          ...((nextSettings as Record<string, unknown>) || {}),
        };
        params.applyPreviewVolume();
      },
      setBindings: (bindings: Record<string, string>) => {
        params.state.bindings = bindings;
      },
      setStatusError: params.setStatusError,
      disableInteractiveControls: () => {
        if (params.refreshButton instanceof HTMLButtonElement) {
          params.refreshButton.disabled = true;
        }
        if (params.searchInput instanceof HTMLInputElement) {
          params.searchInput.disabled = true;
        }
        if (params.stopPlaybackButton instanceof HTMLButtonElement) {
          params.stopPlaybackButton.disabled = true;
        }
        if (params.addLinkButton instanceof HTMLButtonElement) {
          params.addLinkButton.disabled = true;
        }
      },
      initializePreviewLayoutObserver: () => {
        if (typeof ResizeObserver !== 'function' || !(params.previewStageNode instanceof HTMLElement)) {
          return;
        }

        params.state.previewLayoutObserver = new ResizeObserver(() => {
          params.schedulePreviewLayoutRefresh();
        });
        params.refreshPreviewLayoutObserverTargets();
      },
      loadItemsAndRender: params.loadItemsAndRender,
    });
  }

  return {
    bootstrapBoardRuntime,
  };
}
