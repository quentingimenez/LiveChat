function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function isTextVisible(showTextValue) {
    return showTextValue !== false;
}
// Flex alignment of the media frame in the display zone, as
// [align-items (vertical), justify-content (horizontal)].
const LAYOUT_ALIGNMENTS = {
    center: ['center', 'center'],
    top: ['flex-start', 'center'],
    bottom: ['flex-end', 'center'],
    left: ['center', 'flex-start'],
    right: ['center', 'flex-end'],
    'top-left': ['flex-start', 'flex-start'],
    'top-right': ['flex-start', 'flex-end'],
    'bottom-left': ['flex-end', 'flex-start'],
    'bottom-right': ['flex-end', 'flex-end'],
};
const FULL_ZONE = { x: 0, y: 0, width: 1, height: 1 };
function isFraction(value) {
    return typeof value === 'number' && Number.isFinite(value) && value >= 0 && value <= 1;
}
// The shell already normalizes the zone; anything else falls back to the
// whole screen rather than hiding the media.
function resolveZone(zoneValue) {
    const zone = toRecord(zoneValue);
    if (![zone.x, zone.y, zone.width, zone.height].every(isFraction) ||
        zone.width === 0 ||
        zone.height === 0) {
        return { ...FULL_ZONE };
    }
    return {
        x: zone.x,
        y: zone.y,
        width: Math.min(zone.width, 1 - zone.x),
        height: Math.min(zone.height, 1 - zone.y),
    };
}
export function resolveLayout(layoutValue) {
    const layout = toRecord(layoutValue);
    const align = Object.prototype.hasOwnProperty.call(LAYOUT_ALIGNMENTS, layout.align)
        ? layout.align
        : 'center';
    const [alignItems, justifyContent] = LAYOUT_ALIGNMENTS[align];
    return { zone: resolveZone(layout.zone), align, alignItems, justifyContent };
}
function percent(fraction) {
    return Math.round(fraction * 1_000_000) / 10_000;
}
export function applyOverlayLayout(params) {
    const root = params.root;
    if (!(root instanceof HTMLElement)) {
        return;
    }
    const layout = resolveLayout(params.layout);
    root.style.setProperty('--overlay-zone-left', `${percent(layout.zone.x)}vw`);
    root.style.setProperty('--overlay-zone-top', `${percent(layout.zone.y)}vh`);
    root.style.setProperty('--overlay-zone-width', `${percent(layout.zone.width)}vw`);
    root.style.setProperty('--overlay-zone-height', `${percent(layout.zone.height)}vh`);
    root.style.setProperty('--overlay-layout-align-items', layout.alignItems);
    root.style.setProperty('--overlay-layout-justify-content', layout.justifyContent);
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
