interface BootstrapBoardParams {
  getConfig: () => Promise<unknown>;
  getMemeBindings: () => Promise<unknown>;
  onSettings: (listener: (nextSettings: unknown) => void) => void;
  setConfig: (config: unknown) => void;
  mergeConfig: (nextSettings: unknown) => void;
  setBindings: (bindings: Record<string, string>) => void;
  setStatusError: (message: string) => void;
  disableInteractiveControls: () => void;
  initializePreviewLayoutObserver: () => void;
  loadItemsAndRender: () => Promise<void>;
}

export interface BoardLegacyBootstrapUtils {
  bootstrapBoard(params: BootstrapBoardParams): Promise<void>;
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function isPairedConfig(config: unknown): boolean {
  const record = toRecord(config);
  const serverUrl = toTrimmedString(record.serverUrl);
  const clientToken = toTrimmedString(record.clientToken);
  const guildId = toTrimmedString(record.guildId);

  return Boolean(serverUrl && clientToken && guildId);
}

function toBindingsRecord(value: unknown): Record<string, string> {
  const record = toRecord(value);
  const bindingsSource = toRecord(record.bindings);
  const bindings: Record<string, string> = {};

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

export function createBoardLegacyBootstrapUtils(): BoardLegacyBootstrapUtils {
  async function bootstrapBoard(params: BootstrapBoardParams): Promise<void> {
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
