import type { MenuItemConstructorOptions } from 'electron';
import type { ConfigService, OverlayRuntimeConfig } from '../services/config-service';
import type { ConnectionStateService } from '../services/connection-state-service';
import type { DisplayLike, DisplayService } from '../services/display-service';
import type { OverlayPeersService } from '../services/overlay-peers-service';
import { buildVolumeMenuItems as buildTrayVolumeMenuItems } from '../services/tray-volume-service';

interface CreateRuntimeCoreFacadeServiceOptions {
  configService: ConfigService;
  displayService: DisplayService;
  connectionStateService: ConnectionStateService;
  overlayPeersService: OverlayPeersService;
  onVolumeSelected: (volume: number) => void;
  onOverlayPeersChanged: () => void;
}

export interface RuntimeCoreFacadeService {
  normalizeVolume(volume: unknown): number;
  normalizeGuestMode(value: unknown): boolean;
  normalizeMemeBindings(candidate: unknown): Record<string, string>;
  normalizeServerUrl(serverUrl: string): string;
  loadConfig(): OverlayRuntimeConfig;
  saveConfig(nextValues: Partial<OverlayRuntimeConfig>): OverlayRuntimeConfig;
  hasPairingConfig(config?: OverlayRuntimeConfig): boolean;
  isGuestModeEnabled(config?: OverlayRuntimeConfig): boolean;
  getTargetDisplay(): DisplayLike;
  getDisplayConfigUpdate(
    display: DisplayLike,
    index: number
  ): {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  };
  formatDisplayMenuLabel(display: DisplayLike, index: number): string;
  getConnectionStateLabel(): string;
  setOverlayConnectionState(nextState: string, reason?: string): void;
  getOverlayConnectionReason(): string;
  setConnectedOverlayPeers(peers: unknown): void;
  buildTrayTooltip(config?: OverlayRuntimeConfig): string;
  buildVolumeMenuItems(currentVolume: number): MenuItemConstructorOptions[];
}

export function createRuntimeCoreFacadeService(options: CreateRuntimeCoreFacadeServiceOptions): RuntimeCoreFacadeService {
  const { configService, displayService, connectionStateService, overlayPeersService, onVolumeSelected, onOverlayPeersChanged } = options;

  function normalizeVolume(volume: unknown): number {
    return configService.normalizeVolume(volume);
  }

  function normalizeGuestMode(value: unknown): boolean {
    return configService.normalizeGuestMode(value);
  }

  function normalizeMemeBindings(candidate: unknown): Record<string, string> {
    return configService.normalizeMemeBindings(candidate);
  }

  function normalizeServerUrl(serverUrl: string): string {
    return configService.normalizeServerUrl(serverUrl);
  }

  function loadConfig(): OverlayRuntimeConfig {
    return configService.loadConfig();
  }

  function saveConfig(nextValues: Partial<OverlayRuntimeConfig>): OverlayRuntimeConfig {
    return configService.saveConfig(nextValues);
  }

  function hasPairingConfig(config: OverlayRuntimeConfig = loadConfig()): boolean {
    return configService.hasPairingConfig(config);
  }

  function isGuestModeEnabled(config: OverlayRuntimeConfig = loadConfig()): boolean {
    return configService.isGuestModeEnabled(config);
  }

  function getTargetDisplay(): DisplayLike {
    return displayService.getTargetDisplay(loadConfig());
  }

  function getDisplayConfigUpdate(
    display: DisplayLike,
    index: number
  ): {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  } {
    return displayService.getDisplayConfigUpdate(display, index);
  }

  function formatDisplayMenuLabel(display: DisplayLike, index: number): string {
    return displayService.formatDisplayMenuLabel(display, index);
  }

  function getConnectionStateLabel(): string {
    return connectionStateService.getConnectionStateLabel();
  }

  function setOverlayConnectionState(nextState: string, reason = ''): void {
    connectionStateService.setConnectionState(nextState, reason);
  }

  function getOverlayConnectionReason(): string {
    return connectionStateService.getConnectionReason();
  }

  function setConnectedOverlayPeers(peers: unknown): void {
    overlayPeersService.setConnectedOverlayPeers(peers);
    onOverlayPeersChanged();
  }

  function buildTrayTooltip(config: OverlayRuntimeConfig = loadConfig()): string {
    return overlayPeersService.buildTrayTooltip({
      config,
      connectionStateLabel: getConnectionStateLabel()
    });
  }

  function buildVolumeMenuItems(currentVolume: number): MenuItemConstructorOptions[] {
    return buildTrayVolumeMenuItems({
      currentVolume,
      normalizeVolume,
      onVolumeSelected
    });
  }

  return {
    normalizeVolume,
    normalizeGuestMode,
    normalizeMemeBindings,
    normalizeServerUrl,
    loadConfig,
    saveConfig,
    hasPairingConfig,
    isGuestModeEnabled,
    getTargetDisplay,
    getDisplayConfigUpdate,
    formatDisplayMenuLabel,
    getConnectionStateLabel,
    setOverlayConnectionState,
    getOverlayConnectionReason,
    setConnectedOverlayPeers,
    buildTrayTooltip,
    buildVolumeMenuItems
  };
}
