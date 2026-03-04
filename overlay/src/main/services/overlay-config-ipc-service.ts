import type { OverlayRuntimeConfig } from './config-service';

interface CreateOverlayConfigIpcServiceOptions {
  loadConfig: () => OverlayRuntimeConfig;
  isGuestModeEnabled: (config?: OverlayRuntimeConfig) => boolean;
  normalizeMemeBindings: (candidate: unknown) => Record<string, string>;
}

export interface OverlayConfigIpcService {
  getConfig(): {
    serverUrl: string | null;
    clientToken: string | null;
    guildId: string | null;
    clientId: string | null;
    guestMode: boolean;
    showText: boolean;
    volume: number;
    memeBindings: Record<string, string>;
  };
}

export function createOverlayConfigIpcService(options: CreateOverlayConfigIpcServiceOptions): OverlayConfigIpcService {
  const { loadConfig, isGuestModeEnabled, normalizeMemeBindings } = options;

  function getConfig(): {
    serverUrl: string | null;
    clientToken: string | null;
    guildId: string | null;
    clientId: string | null;
    guestMode: boolean;
    showText: boolean;
    volume: number;
    memeBindings: Record<string, string>;
  } {
    const cfg = loadConfig();

    return {
      serverUrl: cfg.serverUrl,
      clientToken: cfg.clientToken,
      guildId: cfg.guildId,
      clientId: cfg.clientId,
      guestMode: isGuestModeEnabled(cfg),
      showText: cfg.showText,
      volume: cfg.volume,
      memeBindings: normalizeMemeBindings(cfg.memeBindings)
    };
  }

  return {
    getConfig
  };
}
