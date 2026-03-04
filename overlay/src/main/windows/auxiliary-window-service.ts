import type { BrowserWindow } from 'electron';
import { createBoardWindowInstance, createPairingWindowInstance } from './window-factory';

interface CreateAuxiliaryWindowServiceOptions {
  appIconPath: string;
  preloadPath: string;
  pairingHtmlPath: string;
  boardHtmlPath: string;
  onBoardReadyToShow: () => void;
}

export interface AuxiliaryWindowService {
  getPairingWindow(): BrowserWindow | null;
  getBoardWindow(): BrowserWindow | null;
  createPairingWindow(): void;
  closePairingWindow(): void;
  createBoardWindow(): void;
  destroyBoardWindow(): void;
}

export function createAuxiliaryWindowService(options: CreateAuxiliaryWindowServiceOptions): AuxiliaryWindowService {
  const { appIconPath, preloadPath, pairingHtmlPath, boardHtmlPath, onBoardReadyToShow } = options;

  let pairingWindow: BrowserWindow | null = null;
  let boardWindow: BrowserWindow | null = null;

  function createPairingWindow(): void {
    if (pairingWindow && !pairingWindow.isDestroyed()) {
      pairingWindow.focus();
      return;
    }

    pairingWindow = createPairingWindowInstance({
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
    if (boardWindow && !boardWindow.isDestroyed()) {
      boardWindow.focus();
      return;
    }

    boardWindow = createBoardWindowInstance({
      appIconPath,
      preloadPath,
      htmlPath: boardHtmlPath
    });

    boardWindow.once('ready-to-show', () => {
      onBoardReadyToShow();
    });

    boardWindow.on('closed', () => {
      boardWindow = null;
    });
  }

  function destroyBoardWindow(): void {
    if (boardWindow && !boardWindow.isDestroyed()) {
      boardWindow.destroy();
    }

    boardWindow = null;
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
