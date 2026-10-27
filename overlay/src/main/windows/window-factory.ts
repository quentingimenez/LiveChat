import { BrowserWindow } from 'electron';

interface OverlayBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface OverlayWindowFactoryOptions {
  bounds: OverlayBounds;
  appIconPath: string;
  preloadPath: string;
  htmlPath: string;
}

export interface PairingWindowFactoryOptions {
  appIconPath: string;
  preloadPath: string;
  htmlPath: string;
}

export interface BoardWindowFactoryOptions {
  appIconPath: string;
  preloadPath: string;
  htmlPath: string;
}

export function createOverlayWindowInstance(options: OverlayWindowFactoryOptions): BrowserWindow {
  const { bounds, appIconPath, preloadPath, htmlPath } = options;

  const overlayWindow = new BrowserWindow({
    width: bounds.width,
    height: bounds.height,
    x: bounds.x,
    y: bounds.y,
    transparent: true,
    frame: false,
    alwaysOnTop: true,
    focusable: false,
    hasShadow: false,
    skipTaskbar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: preloadPath,
      backgroundThrottling: false
    }
  });

  overlayWindow.loadFile(htmlPath);
  overlayWindow.setIgnoreMouseEvents(true, { forward: true });

  return overlayWindow;
}

export function createPairingWindowInstance(options: PairingWindowFactoryOptions): BrowserWindow {
  const { appIconPath, preloadPath, htmlPath } = options;

  const pairingWindow = new BrowserWindow({
    width: 460,
    height: 580,
    resizable: false,
    autoHideMenuBar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: preloadPath
    }
  });

  pairingWindow.loadFile(htmlPath);

  return pairingWindow;
}

export function createBoardWindowInstance(options: BoardWindowFactoryOptions): BrowserWindow {
  const { appIconPath, preloadPath, htmlPath } = options;

  const boardWindow = new BrowserWindow({
    width: 1180,
    height: 780,
    minWidth: 960,
    minHeight: 640,
    autoHideMenuBar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: preloadPath
    }
  });

  boardWindow.loadFile(htmlPath);

  return boardWindow;
}
