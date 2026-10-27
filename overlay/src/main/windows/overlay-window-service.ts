import type { BrowserWindow } from 'electron';
import type { DisplayLike } from '../services/display-service';
import { createOverlayWindowInstance } from './window-factory';

interface CreateOverlayWindowServiceOptions {
  appIconPath: string;
  preloadPath: string;
  htmlPath: string;
  getTargetDisplay: () => DisplayLike;
  getAllDisplays: () => DisplayLike[];
  saveDisplaySelection: (display: DisplayLike, index: number) => void;
  onReadyToShow: () => void;
  onClosed: () => void;
  keepOnTopIntervalMs?: number;
}

export interface OverlayWindowService {
  getOverlayWindow(): BrowserWindow | null;
  createOverlayWindow(): void;
  destroyOverlayWindow(): void;
  moveOverlayWindowToDisplay(display: DisplayLike): void;
  stopKeepOnTopLoop(): void;
}

const DEFAULT_KEEP_ON_TOP_INTERVAL_MS = 2000;

export function createOverlayWindowService(options: CreateOverlayWindowServiceOptions): OverlayWindowService {
  const {
    appIconPath,
    preloadPath,
    htmlPath,
    getTargetDisplay,
    getAllDisplays,
    saveDisplaySelection,
    onReadyToShow,
    onClosed,
    keepOnTopIntervalMs = DEFAULT_KEEP_ON_TOP_INTERVAL_MS
  } = options;

  let overlayWindow: BrowserWindow | null = null;
  let keepOnTopInterval: ReturnType<typeof setInterval> | null = null;

  function stopKeepOnTopLoop(): void {
    if (keepOnTopInterval) {
      clearInterval(keepOnTopInterval);
      keepOnTopInterval = null;
    }
  }

  function startKeepOnTopLoop(): void {
    stopKeepOnTopLoop();

    keepOnTopInterval = setInterval(() => {
      if (overlayWindow && !overlayWindow.isDestroyed()) {
        overlayWindow.setAlwaysOnTop(true, 'screen-saver');
      }
    }, keepOnTopIntervalMs);
  }

  function createOverlayWindow(): void {
    if (overlayWindow && !overlayWindow.isDestroyed()) {
      return;
    }

    const targetDisplay = getTargetDisplay();
    const displays = getAllDisplays();
    const displayIndex = displays.findIndex((display) => display.id === targetDisplay.id);

    saveDisplaySelection(targetDisplay, displayIndex);

    const { width, height, x, y } = targetDisplay.bounds;
    overlayWindow = createOverlayWindowInstance({
      bounds: {
        width,
        height,
        x,
        y
      },
      appIconPath,
      preloadPath,
      htmlPath
    });

    overlayWindow.once('ready-to-show', () => {
      if (!overlayWindow || overlayWindow.isDestroyed()) {
        return;
      }

      overlayWindow.show();
      onReadyToShow();
    });

    overlayWindow.on('closed', () => {
      overlayWindow = null;
      stopKeepOnTopLoop();
      onClosed();
    });

    startKeepOnTopLoop();
  }

  function destroyOverlayWindow(): void {
    stopKeepOnTopLoop();

    if (overlayWindow && !overlayWindow.isDestroyed()) {
      overlayWindow.destroy();
    }

    overlayWindow = null;
  }

  function moveOverlayWindowToDisplay(display: DisplayLike): void {
    if (!overlayWindow || overlayWindow.isDestroyed()) {
      return;
    }

    overlayWindow.setBounds({
      x: display.bounds.x,
      y: display.bounds.y,
      width: display.bounds.width,
      height: display.bounds.height
    });
  }

  return {
    getOverlayWindow(): BrowserWindow | null {
      return overlayWindow;
    },
    createOverlayWindow,
    destroyOverlayWindow,
    moveOverlayWindowToDisplay,
    stopKeepOnTopLoop
  };
}
