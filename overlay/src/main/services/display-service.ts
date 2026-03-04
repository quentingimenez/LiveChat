import type { OverlayRuntimeConfig } from './config-service';

interface DisplayBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface DisplayLike {
  id: number;
  bounds: DisplayBounds;
  label?: string;
}

interface CreateDisplayServiceOptions {
  getAllDisplays: () => DisplayLike[];
  getPrimaryDisplay: () => DisplayLike;
}

export interface DisplayService {
  buildDisplayKey(display: DisplayLike, index: number): string;
  getDisplayLabel(display: DisplayLike | null | undefined): string | null;
  getDisplayConfigUpdate(display: DisplayLike, index: number): {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  };
  formatDisplayMenuLabel(display: DisplayLike, index: number): string;
  getTargetDisplay(config: OverlayRuntimeConfig): DisplayLike;
}

export function createDisplayService(options: CreateDisplayServiceOptions): DisplayService {
  const { getAllDisplays, getPrimaryDisplay } = options;

  function buildDisplayKey(display: DisplayLike, index: number): string {
    const safeIndex = Number.isInteger(index) ? index : -1;
    const { x, y, width, height } = display.bounds;
    return `${x},${y},${width},${height},${safeIndex}`;
  }

  function getDisplayLabel(display: DisplayLike | null | undefined): string | null {
    const label = `${display?.label || ''}`.trim();
    return label || null;
  }

  function getDisplayConfigUpdate(display: DisplayLike, index: number): {
    displayId: number;
    displayIndex: number;
    displayKey: string;
    displayLabel: string | null;
  } {
    return {
      displayId: display.id,
      displayIndex: index,
      displayKey: buildDisplayKey(display, index),
      displayLabel: getDisplayLabel(display)
    };
  }

  function formatDisplayMenuLabel(display: DisplayLike, index: number): string {
    const label = getDisplayLabel(display);
    const nameSuffix = label ? ` - ${label}` : '';
    return `Écran ${index + 1}${nameSuffix}`;
  }

  function getTargetDisplay(config: OverlayRuntimeConfig): DisplayLike {
    const displays = getAllDisplays();

    if (config.displayId !== null) {
      const byId = displays.find((display) => display.id === config.displayId);
      if (byId) {
        return byId;
      }
    }

    if (typeof config.displayKey === 'string' && config.displayKey.trim()) {
      const byKey = displays.find((display, index) => buildDisplayKey(display, index) === config.displayKey);
      if (byKey) {
        return byKey;
      }
    }

    if (Number.isInteger(config.displayIndex)) {
      const displayIndex = config.displayIndex as number;
      if (displays[displayIndex]) {
        return displays[displayIndex];
      }
    }

    return getPrimaryDisplay();
  }

  return {
    buildDisplayKey,
    getDisplayLabel,
    getDisplayConfigUpdate,
    formatDisplayMenuLabel,
    getTargetDisplay
  };
}
