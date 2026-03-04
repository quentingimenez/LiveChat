interface OverlayConfigRecord extends Record<string, unknown> {
  showText?: unknown;
}

interface ApplyOverlaySettingsParams {
  currentConfig: OverlayConfigRecord;
  nextSettings: unknown;
  onHideText: () => void;
  applyVolume: () => void;
}

interface BootstrapOverlayParams {
  getConfig: () => Promise<unknown>;
  getCurrentConfig: () => OverlayConfigRecord;
  setConfig: (nextConfig: OverlayConfigRecord) => void;
  registerOnPlay: (handler: (payload: unknown) => void) => void;
  registerOnStop: (handler: () => void) => void;
  registerOnSettings: (handler: (settings: unknown) => void) => void;
  onPlay: (payload: unknown) => void;
  onStop: () => void;
  onSettings: (settings: unknown) => void;
  addWindowResizeListener: (handler: () => void) => void;
  scheduleMediaHeaderLayout: () => void;
  ensureTwitterWidgets: () => Promise<unknown>;
  rendererReady: () => void;
}

export interface OverlayLegacyBootstrapUtils {
  applyOverlaySettings(params: ApplyOverlaySettingsParams): OverlayConfigRecord;
  bootstrapOverlay(params: BootstrapOverlayParams): Promise<void>;
}

function toRecord(value: unknown): OverlayConfigRecord {
  return typeof value === 'object' && value !== null ? (value as OverlayConfigRecord) : {};
}

function isTextVisible(showTextValue: unknown): boolean {
  return showTextValue !== false;
}

export function createOverlayLegacyBootstrapUtils(): OverlayLegacyBootstrapUtils {
  function applyOverlaySettings(params: ApplyOverlaySettingsParams): OverlayConfigRecord {
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

  async function bootstrapOverlay(params: BootstrapOverlayParams): Promise<void> {
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
