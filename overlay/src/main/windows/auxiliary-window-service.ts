import type { BrowserWindow } from 'electron';
import { createBoardWindowInstance, createPairingWindowInstance } from './window-factory';

interface CreateAuxiliaryWindowServiceOptions {
  appIconPath: string;
  preloadPath: string;
  pairingHtmlPath: string;
  boardHtmlPath: string;
  onBoardReadyToShow: () => void;
  createPairingWindowFactory?: (options: { appIconPath: string; preloadPath: string; htmlPath: string }) => BrowserWindow;
  createBoardWindowFactory?: (options: { appIconPath: string; preloadPath: string; htmlPath: string }) => BrowserWindow;
}

export interface AuxiliaryWindowService {
  getPairingWindow(): BrowserWindow | null;
  getBoardWindow(): BrowserWindow | null;
  createPairingWindow(): void;
  closePairingWindow(): void;
  createBoardWindow(): void;
  destroyBoardWindow(): void;
}

function isBoardWindowReusable(window: BrowserWindow | null): window is BrowserWindow {
  if (!window || window.isDestroyed()) {
    return false;
  }

  const webContentsLike = window.webContents as unknown as {
    isDestroyed?: () => boolean;
    isCrashed?: () => boolean;
  };

  if (typeof webContentsLike.isDestroyed === 'function' && webContentsLike.isDestroyed()) {
    return false;
  }

  if (typeof webContentsLike.isCrashed === 'function' && webContentsLike.isCrashed()) {
    return false;
  }

  return true;
}

export function createAuxiliaryWindowService(options: CreateAuxiliaryWindowServiceOptions): AuxiliaryWindowService {
  const {
    appIconPath,
    preloadPath,
    pairingHtmlPath,
    boardHtmlPath,
    onBoardReadyToShow,
    createPairingWindowFactory = createPairingWindowInstance,
    createBoardWindowFactory = createBoardWindowInstance
  } = options;

  let pairingWindow: BrowserWindow | null = null;
  let boardWindow: BrowserWindow | null = null;
  let allowBoardWindowClose = false;

  function createPairingWindow(): void {
    if (pairingWindow && !pairingWindow.isDestroyed()) {
      pairingWindow.focus();
      return;
    }

    pairingWindow = createPairingWindowFactory({
      appIconPath,
      preloadPath,
      htmlPath: pairingHtmlPath
    });

    pairingWindow.on('closed', () => {
      pairingWindow = null;
    });
  }

  function closePairingWindow(): void {
    if (pairingWindow && !pairingWindow.isDestroyed()) {
      pairingWindow.close();
    }
  }

  function createBoardWindow(): void {
    const existingBoardWindow = boardWindow;
    if (isBoardWindowReusable(existingBoardWindow)) {
      existingBoardWindow.show();
      existingBoardWindow.focus();
      return;
    }

    if (boardWindow && !boardWindow.isDestroyed()) {
      allowBoardWindowClose = true;
      boardWindow.destroy();
      boardWindow = null;
      allowBoardWindowClose = false;
    }

    boardWindow = createBoardWindowFactory({
      appIconPath,
      preloadPath,
      htmlPath: boardHtmlPath
    });

    boardWindow.on('close', (event) => {
      if (allowBoardWindowClose || !boardWindow || boardWindow.isDestroyed()) {
        return;
      }

      event.preventDefault();
      boardWindow.hide();
    });

    boardWindow.once('ready-to-show', () => {
      onBoardReadyToShow();
    });

    boardWindow.on('closed', () => {
      boardWindow = null;
      allowBoardWindowClose = false;
    });

    boardWindow.webContents.on('render-process-gone', () => {
      if (!boardWindow || boardWindow.isDestroyed()) {
        return;
      }

      allowBoardWindowClose = true;
      boardWindow.destroy();
      boardWindow = null;
      allowBoardWindowClose = false;
    });
  }

  function destroyBoardWindow(): void {
    if (boardWindow && !boardWindow.isDestroyed()) {
      allowBoardWindowClose = true;
      boardWindow.close();

      if (boardWindow && !boardWindow.isDestroyed()) {
        boardWindow.destroy();
      }
    }

    boardWindow = null;
    allowBoardWindowClose = false;
  }

  return {
    getPairingWindow(): BrowserWindow | null {
      return pairingWindow;
    },
    getBoardWindow(): BrowserWindow | null {
      return boardWindow;
    },
    createPairingWindow,
    closePairingWindow,
    createBoardWindow,
    destroyBoardWindow
  };
}
