const test = require('node:test');
const assert = require('node:assert/strict');

const { createAuxiliaryWindowService } = require('../../dist/main/windows/auxiliary-window-service.js');

function createWindowMock() {
  const listeners = new Map();
  const onceListeners = new Map();
  const webContentsListeners = new Map();

  let destroyed = false;
  let showCalls = 0;
  let hideCalls = 0;
  let focusCalls = 0;
  let webContentsDestroyed = false;
  let webContentsCrashed = false;

  function emit(eventName, payload) {
    const eventListeners = listeners.get(eventName) || [];
    for (const listener of eventListeners) {
      listener(payload);
    }

    const eventOnceListeners = onceListeners.get(eventName) || [];
    onceListeners.delete(eventName);
    for (const listener of eventOnceListeners) {
      listener(payload);
    }
  }

  return {
    on(eventName, listener) {
      const current = listeners.get(eventName) || [];
      current.push(listener);
      listeners.set(eventName, current);
    },
    once(eventName, listener) {
      const current = onceListeners.get(eventName) || [];
      current.push(listener);
      onceListeners.set(eventName, current);
    },
    isDestroyed() {
      return destroyed;
    },
    show() {
      showCalls += 1;
    },
    hide() {
      hideCalls += 1;
    },
    focus() {
      focusCalls += 1;
    },
    close() {
      const closeEvent = {
        defaultPrevented: false,
        preventDefault() {
          closeEvent.defaultPrevented = true;
        }
      };

      emit('close', closeEvent);
      if (!closeEvent.defaultPrevented) {
        destroyed = true;
        emit('closed');
      }
    },
    destroy() {
      destroyed = true;
      webContentsDestroyed = true;
      emit('closed');
    },
    emit(eventName, payload) {
      emit(eventName, payload);
    },
    webContents: {
      on(eventName, listener) {
        const current = webContentsListeners.get(eventName) || [];
        current.push(listener);
        webContentsListeners.set(eventName, current);
      },
      emit(eventName, payload) {
        const current = webContentsListeners.get(eventName) || [];
        for (const listener of current) {
          listener(payload);
        }
      },
      isDestroyed() {
        return webContentsDestroyed;
      },
      isCrashed() {
        return webContentsCrashed;
      },
      setCrashed(value) {
        webContentsCrashed = value === true;
      }
    },
    stats() {
      return {
        showCalls,
        hideCalls,
        focusCalls
      };
    }
  };
}

test('auxiliary-window-service keeps board window alive when user closes it', () => {
  const boardWindow = createWindowMock();
  let boardReadyCalls = 0;
  let boardFactoryCalls = 0;

  const service = createAuxiliaryWindowService({
    appIconPath: '/tmp/icon.png',
    preloadPath: '/tmp/preload.js',
    pairingHtmlPath: '/tmp/pairing.html',
    boardHtmlPath: '/tmp/board.html',
    onBoardReadyToShow: () => {
      boardReadyCalls += 1;
    },
    createPairingWindowFactory: () => createWindowMock(),
    createBoardWindowFactory: () => {
      boardFactoryCalls += 1;
      return boardWindow;
    }
  });

  service.createBoardWindow();
  boardWindow.emit('ready-to-show');

  assert.equal(boardFactoryCalls, 1);
  assert.equal(boardReadyCalls, 1);
  assert.equal(service.getBoardWindow(), boardWindow);

  boardWindow.close();
  assert.equal(service.getBoardWindow(), boardWindow);
  assert.equal(boardWindow.stats().hideCalls, 1);
  assert.equal(boardWindow.isDestroyed(), false);

  service.createBoardWindow();
  assert.equal(boardFactoryCalls, 1);
  assert.equal(boardWindow.stats().showCalls, 1);
  assert.equal(boardWindow.stats().focusCalls, 1);

  service.destroyBoardWindow();
  assert.equal(service.getBoardWindow(), null);
  assert.equal(boardWindow.isDestroyed(), true);
});

test('auxiliary-window-service recreates board window when existing renderer is crashed', () => {
  const firstBoardWindow = createWindowMock();
  const secondBoardWindow = createWindowMock();
  let boardFactoryCalls = 0;

  const service = createAuxiliaryWindowService({
    appIconPath: '/tmp/icon.png',
    preloadPath: '/tmp/preload.js',
    pairingHtmlPath: '/tmp/pairing.html',
    boardHtmlPath: '/tmp/board.html',
    onBoardReadyToShow: () => {},
    createPairingWindowFactory: () => createWindowMock(),
    createBoardWindowFactory: () => {
      boardFactoryCalls += 1;
      return boardFactoryCalls === 1 ? firstBoardWindow : secondBoardWindow;
    }
  });

  service.createBoardWindow();
  assert.equal(service.getBoardWindow(), firstBoardWindow);

  firstBoardWindow.webContents.setCrashed(true);
  service.createBoardWindow();

  assert.equal(boardFactoryCalls, 2);
  assert.equal(service.getBoardWindow(), secondBoardWindow);
  assert.equal(firstBoardWindow.isDestroyed(), true);
});
