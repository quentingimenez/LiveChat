import fs from 'node:fs';
import type { Dictionary } from '../../shared/types';

export interface OverlayRuntimeConfig {
  displayId: number | null;
  displayIndex: number | null;
  displayKey: string | null;
  displayLabel: string | null;
  volume: number;
  enabled: boolean;
  autoStart: boolean;
  guestMode: boolean;
  showText: boolean;
  serverUrl: string | null;
  clientToken: string | null;
  clientId: string | null;
  guildId: string | null;
  authorName: string | null;
  deviceName: string | null;
  memeBindings: Dictionary<string>;
}

export interface ConfigService {
  readonly defaultConfig: OverlayRuntimeConfig;
  normalizeVolume(volume: unknown): number;
  normalizeAutoStart(value: unknown): boolean;
  normalizeGuestMode(value: unknown): boolean;
  normalizeMemeBindings(candidate: unknown): Dictionary<string>;
  normalizeServerUrl(serverUrl: string): string;
  loadConfig(): OverlayRuntimeConfig;
  saveConfig(nextValues: Partial<OverlayRuntimeConfig>): OverlayRuntimeConfig;
  hasPairingConfig(config?: OverlayRuntimeConfig): boolean;
  isGuestModeEnabled(config?: OverlayRuntimeConfig): boolean;
}

export interface CreateConfigServiceOptions {
  configPath: string;
  onReadError?: (error: unknown) => void;
  onWriteError?: (error: unknown) => void;
}

const DEFAULT_CONFIG: OverlayRuntimeConfig = {
  displayId: null,
  displayIndex: null,
  displayKey: null,
  displayLabel: null,
  volume: 1,
  enabled: true,
  autoStart: false,
  guestMode: false,
  showText: true,
  serverUrl: null,
  clientToken: null,
  clientId: null,
  guildId: null,
  authorName: null,
  deviceName: null,
  memeBindings: {}
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function createConfigService(options: CreateConfigServiceOptions): ConfigService {
  const { configPath, onReadError, onWriteError } = options;

  function normalizeVolume(volume: unknown): number {
    if (typeof volume !== 'number' || !Number.isFinite(volume)) {
      return DEFAULT_CONFIG.volume;
    }

    return Math.min(1, Math.max(0, volume));
  }

  function normalizeAutoStart(value: unknown): boolean {
    return value === true;
  }

  function normalizeGuestMode(value: unknown): boolean {
    return value === true;
  }

  function normalizeMemeBindings(candidate: unknown): Dictionary<string> {
    if (!isRecord(candidate)) {
      return {};
    }

    const normalized: Dictionary<string> = {};

    for (const [rawAccelerator, rawItemId] of Object.entries(candidate)) {
      const accelerator = `${rawAccelerator || ''}`.trim();
      const itemId = `${rawItemId || ''}`.trim();

      if (!accelerator || !itemId) {
        continue;
      }

      normalized[accelerator] = itemId;
    }

    return normalized;
  }

  function normalizeServerUrl(serverUrl: string): string {
    return `${serverUrl || ''}`.trim().replace(/\/+$/, '');
  }

  function loadConfig(): OverlayRuntimeConfig {
    try {
      if (!fs.existsSync(configPath)) {
        return { ...DEFAULT_CONFIG };
      }

      const parsed = JSON.parse(fs.readFileSync(configPath, 'utf-8')) as unknown;
      const source = isRecord(parsed) ? parsed : {};

      return {
        ...DEFAULT_CONFIG,
        ...source,
        volume: normalizeVolume(source.volume),
        autoStart: normalizeAutoStart(source.autoStart),
        guestMode: normalizeGuestMode(source.guestMode),
        memeBindings: normalizeMemeBindings(source.memeBindings)
      };
    } catch (error) {
      if (onReadError) {
        onReadError(error);
      }
      return { ...DEFAULT_CONFIG };
    }
  }

  function saveConfig(nextValues: Partial<OverlayRuntimeConfig>): OverlayRuntimeConfig {
    try {
      const merged: OverlayRuntimeConfig = {
        ...loadConfig(),
        ...nextValues
      };

      const normalizedMerged: OverlayRuntimeConfig = {
        ...merged,
        volume: normalizeVolume(merged.volume),
        autoStart: normalizeAutoStart(merged.autoStart),
        guestMode: normalizeGuestMode(merged.guestMode),
        memeBindings: normalizeMemeBindings(merged.memeBindings)
      };

      fs.writeFileSync(configPath, JSON.stringify(normalizedMerged, null, 2));
      return normalizedMerged;
    } catch (error) {
      if (onWriteError) {
        onWriteError(error);
      }
      return loadConfig();
    }
  }

  function hasPairingConfig(config: OverlayRuntimeConfig = loadConfig()): boolean {
    return Boolean(config.serverUrl && config.clientToken && config.guildId);
  }

  function isGuestModeEnabled(config: OverlayRuntimeConfig = loadConfig()): boolean {
    return normalizeGuestMode(config.guestMode);
  }

  return {
    defaultConfig: { ...DEFAULT_CONFIG },
    normalizeVolume,
    normalizeAutoStart,
    normalizeGuestMode,
    normalizeMemeBindings,
    normalizeServerUrl,
    loadConfig,
    saveConfig,
    hasPairingConfig,
    isGuestModeEnabled
  };
}
