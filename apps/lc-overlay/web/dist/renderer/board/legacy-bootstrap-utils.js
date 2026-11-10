function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function isPairedConfig(config) {
    const record = toRecord(config);
    const serverUrl = toTrimmedString(record.serverUrl);
    const clientToken = toTrimmedString(record.clientToken);
    const guildId = toTrimmedString(record.guildId);
    return Boolean(serverUrl && clientToken && guildId);
}
function toBindingsRecord(value) {
    const record = toRecord(value);
    const bindingsSource = toRecord(record.bindings);
    const bindings = {};
    for (const [accelerator, mappedItemId] of Object.entries(bindingsSource)) {
        const normalizedAccelerator = toTrimmedString(accelerator);
        const normalizedItemId = toTrimmedString(mappedItemId);
        if (!normalizedAccelerator || !normalizedItemId) {
            continue;
        }
        bindings[normalizedAccelerator] = normalizedItemId;
    }
    return bindings;
}
export function createBoardLegacyBootstrapUtils() {
    async function bootstrapBoard(params) {
        const config = await params.getConfig();
        params.setConfig(config);
        if (!isPairedConfig(config)) {
            params.setStatusError("Overlay non appairé. Ouvre la fenêtre d'appairage puis reconnecte.");
            params.disableInteractiveControls();
            return;
        }
        const bindingsResult = await params.getMemeBindings();
        params.setBindings(toBindingsRecord(bindingsResult));
        params.onSettings((nextSettings) => {
            params.mergeConfig(nextSettings);
        });
        params.initializePreviewLayoutObserver();
        await params.loadItemsAndRender();
    }
    return {
        bootstrapBoard,
    };
}
