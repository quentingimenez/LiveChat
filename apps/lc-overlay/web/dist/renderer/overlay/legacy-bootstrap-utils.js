function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function isTextVisible(showTextValue) {
    return showTextValue !== false;
}
export function createOverlayLegacyBootstrapUtils() {
    function applyOverlaySettings(params) {
        const nextConfig = {
            ...toRecord(params.currentConfig),
            ...toRecord(params.nextSettings),
        };
        if (!isTextVisible(nextConfig.showText)) {
            params.onHideText();
        }
        params.applyVolume();
        return nextConfig;
    }
    async function bootstrapOverlay(params) {
        const loadedConfig = toRecord(await params.getConfig());
        params.setConfig({
            ...toRecord(params.getCurrentConfig()),
            ...loadedConfig,
        });
        params.registerOnPlay(params.onPlay);
        params.registerOnStop(params.onStop);
        params.registerOnSettings(params.onSettings);
        params.addWindowResizeListener(() => {
            params.scheduleMediaHeaderLayout();
        });
        void params.ensureTwitterWidgets().catch(() => undefined);
        params.rendererReady();
    }
    return {
        applyOverlaySettings,
        bootstrapOverlay,
    };
}
