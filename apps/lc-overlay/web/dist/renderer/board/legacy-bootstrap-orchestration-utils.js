export function createBoardLegacyBootstrapOrchestrationUtils() {
    async function bootstrapBoardRuntime(params) {
        await params.bootstrapBoard({
            getConfig: params.getConfig,
            getMemeBindings: params.getMemeBindings,
            onSettings: params.onSettings,
            setConfig: (config) => {
                params.state.config = config;
            },
            mergeConfig: (nextSettings) => {
                params.state.config = {
                    ...(params.state.config || {}),
                    ...(nextSettings || {}),
                };
                params.applyPreviewVolume();
            },
            setBindings: (bindings) => {
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
