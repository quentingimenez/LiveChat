function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function isTextVisible(showTextValue) {
    return showTextValue !== false;
}
function clampNumber(value, minimum, maximum, fallback) {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
        return fallback;
    }
    return Math.min(maximum, Math.max(minimum, Math.round(value)));
}
function resolveLayout(layoutValue) {
    const layout = toRecord(layoutValue);
    const anchors = {
        center: ['center', 'center'],
        top: ['flex-start', 'center'],
        bottom: ['flex-end', 'center'],
        'top-left': ['flex-start', 'flex-start'],
        'top-right': ['flex-start', 'flex-end'],
        'bottom-left': ['flex-end', 'flex-start'],
        'bottom-right': ['flex-end', 'flex-end'],
    };
    const anchor = Object.prototype.hasOwnProperty.call(anchors, layout.anchor) ? layout.anchor : 'center';
    const [alignItems, justifyContent] = anchors[anchor];
    return {
        anchor,
        alignItems,
        justifyContent,
        maxWidth: clampNumber(layout.maxWidth, 20, 100, 100),
        maxHeight: clampNumber(layout.maxHeight, 20, 100, 100),
        margin: clampNumber(layout.margin, 0, 200, 24),
    };
}
function applyOverlayLayout(params) {
    const root = params.root;
    if (!(root instanceof HTMLElement)) {
        return;
    }
    const layout = resolveLayout(params.layout);
    root.style.setProperty('--overlay-layout-anchor', layout.anchor);
    root.style.setProperty('--overlay-layout-align-items', layout.alignItems);
    root.style.setProperty('--overlay-layout-justify-content', layout.justifyContent);
    root.style.setProperty('--overlay-layout-max-width', `${layout.maxWidth}vw`);
    root.style.setProperty('--overlay-layout-max-height', `${layout.maxHeight}vh`);
    root.style.setProperty('--overlay-safe-margin', `${layout.margin}px`);
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
        applyOverlayLayout({
            root: params.overlayRoot,
            layout: nextConfig.layout,
        });
        params.applyVolume();
        return nextConfig;
    }
    async function bootstrapOverlay(params) {
        const loadedConfig = toRecord(await params.getConfig());
        const nextConfig = {
            ...toRecord(params.getCurrentConfig()),
            ...loadedConfig,
        };
        params.setConfig(nextConfig);
        applyOverlayLayout({
            root: params.overlayRoot,
            layout: nextConfig.layout,
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
        applyOverlayLayout,
        applyOverlaySettings,
        bootstrapOverlay,
    };
}
