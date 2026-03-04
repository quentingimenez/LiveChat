const test = require('node:test');
const assert = require('node:assert/strict');

const { registerOverlayIpcHandlers } = require('../../dist/main/ipc/register-overlay-ipc-handlers.js');
const { IPC_CHANNELS } = require('../../dist/shared/ipc.js');
const { OVERLAY_SOCKET_EVENTS } = require('../../dist/shared/socket-events.js');

function createIpcMainMock() {
  const handles = new Map();
  const listeners = new Map();

  return {
    handles,
    listeners,
    handle(channel, handler) {
      handles.set(channel, handler);
    },
    on(channel, listener) {
      listeners.set(channel, listener);
    },
  };
}

function createHarness() {
  const ipcMain = createIpcMainMock();
  const applyCalls = [];
  const triggerCalls = [];
  let stopCalls = 0;
  let settingsCalls = 0;
  const pendingStatePayloads = [];
  const pendingStopPayloads = [];
  let guestModeEnabled = false;

  const socket = {
    connected: true,
    emitted: [],
    emit(event, payload) {
      this.emitted.push({ event, payload });
    },
  };

  const config = {
    serverUrl: 'https://api.livechat.test',
    clientToken: 'token-1',
    guildId: 'guild-1',
    clientId: 'client-1',
    guestMode: false,
    showText: true,
    volume: 0.75,
    memeBindings: {
      'Ctrl+1': 'item-a',
      '': 'invalid',
    },
  };

  const normalizeMemeBindings = (candidate) => {
    if (!candidate || typeof candidate !== 'object') {
      return {};
    }

    return Object.fromEntries(
      Object.entries(candidate).filter(([key, value]) => `${key || ''}`.trim() && `${value || ''}`.trim())
    );
  };

  registerOverlayIpcHandlers({
    logger: {
      info() {},
      warn() {},
      error() {}
    },
    ipcMain,
    loadConfig: () => config,
    saveConfig: () => config,
    hasPairingConfig: () => true,
    isGuestModeEnabled: (cfg) => (cfg ? cfg.guestMode === true : guestModeEnabled),
    normalizeMemeBindings,
    normalizeServerUrl: (url) => `${url || ''}`.trim(),
    applyMemeBindings(nextBindings, options) {
      applyCalls.push({ nextBindings, options });
      return {
        ok: true,
        appliedBindings: nextBindings,
        failedAccelerators: ['Ctrl+9'],
      };
    },
    emitMemeTriggerSignal(itemId, trigger) {
      triggerCalls.push({ itemId, trigger });
      return { ok: true };
    },
    emitManualStopSignal() {
      stopCalls += 1;
      return { ok: true, reason: 'manual' };
    },
    sendOverlaySettingsToRenderer() {
      settingsCalls += 1;
    },
    getOverlaySocket: () => socket,
    setPendingPlaybackStatePayload(payload) {
      pendingStatePayloads.push(payload);
    },
    setPendingPlaybackStopPayload(payload) {
      pendingStopPayloads.push(payload);
    },
  });

  return {
    ipcMain,
    socket,
    applyCalls,
    triggerCalls,
    pendingStatePayloads,
    pendingStopPayloads,
    setGuestModeEnabled(value) {
      guestModeEnabled = value === true;
    },
    getCounts() {
      return {
        stopCalls,
        settingsCalls,
      };
    },
  };
}

test('register-overlay-ipc-handlers wires invoke channels and preserves response format', async () => {
  const harness = createHarness();

  assert.equal(harness.ipcMain.handles.has(IPC_CHANNELS.overlayGetConfig), true);
  assert.equal(harness.ipcMain.handles.has(IPC_CHANNELS.memeBoardGetBindings), true);
  assert.equal(harness.ipcMain.handles.has(IPC_CHANNELS.memeBoardSetBindings), true);
  assert.equal(harness.ipcMain.handles.has(IPC_CHANNELS.memeBoardTrigger), true);
  assert.equal(harness.ipcMain.handles.has(IPC_CHANNELS.memeBoardStop), true);

  const overlayConfig = await harness.ipcMain.handles.get(IPC_CHANNELS.overlayGetConfig)(null, undefined);
  assert.deepEqual(overlayConfig, {
    serverUrl: 'https://api.livechat.test',
    clientToken: 'token-1',
    guildId: 'guild-1',
    clientId: 'client-1',
    guestMode: false,
    showText: true,
    volume: 0.75,
    memeBindings: {
      'Ctrl+1': 'item-a',
    },
  });

  const bindings = await harness.ipcMain.handles.get(IPC_CHANNELS.memeBoardGetBindings)(null, undefined);
  assert.deepEqual(bindings, {
    bindings: {
      'Ctrl+1': 'item-a',
    },
  });

  const setResult = await harness.ipcMain.handles.get(IPC_CHANNELS.memeBoardSetBindings)(null, {
    bindings: {
      'Ctrl+2': 'item-b',
    },
  });
  assert.deepEqual(harness.applyCalls[0], {
    nextBindings: {
      'Ctrl+2': 'item-b',
    },
    options: {
      strict: true,
      persist: true,
    },
  });
  assert.equal(setResult.ok, true);
  assert.deepEqual(setResult.failedAccelerators, ['Ctrl+9']);

  await harness.ipcMain.handles.get(IPC_CHANNELS.memeBoardTrigger)(null, {
    itemId: 'item-42',
    trigger: 'button',
  });
  assert.deepEqual(harness.triggerCalls[0], {
    itemId: 'item-42',
    trigger: 'button',
  });

  const stopResult = await harness.ipcMain.handles.get(IPC_CHANNELS.memeBoardStop)(null, undefined);
  assert.deepEqual(stopResult, { ok: true, reason: 'manual' });
  assert.equal(harness.getCounts().stopCalls, 1);
});

test('register-overlay-ipc-handlers forwards overlay listeners and buffers when socket is offline', () => {
  const harness = createHarness();

  harness.socket.connected = false;
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackState)(null, {
    jobId: '  job-7 ',
    state: ' playing ',
  });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackStop)(null, {
    jobId: '  stop-3 ',
  });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayError)(null, { code: 'ERR_OFFLINE' });

  assert.equal(harness.socket.emitted.length, 0);
  assert.deepEqual(harness.pendingStatePayloads, [
    {
      jobId: '  job-7 ',
      state: ' playing ',
    },
  ]);
  assert.deepEqual(harness.pendingStopPayloads, [{ jobId: 'stop-3' }]);

  harness.socket.connected = true;
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayError)(null, { code: 'ERR_CONNECTED' });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackState)(null, {
    jobId: 'job-8',
    state: 'done',
    remainingMs: 0,
  });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackStop)(null, {});
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayRendererReady)(null, undefined);

  assert.deepEqual(harness.socket.emitted, [
    { event: OVERLAY_SOCKET_EVENTS.ERROR, payload: { code: 'ERR_CONNECTED' } },
    {
      event: OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE,
      payload: {
        jobId: 'job-8',
        state: 'done',
        remainingMs: 0,
      },
    },
    { event: OVERLAY_SOCKET_EVENTS.STOP, payload: { jobId: 'unknown' } },
  ]);
  assert.equal(harness.getCounts().settingsCalls, 1);
});

test('register-overlay-ipc-handlers ignores forwarding in guest mode', () => {
  const harness = createHarness();
  harness.setGuestModeEnabled(true);

  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayError)(null, { code: 'ERR_GUEST' });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackState)(null, {
    jobId: 'guest-1',
    state: 'paused',
  });
  harness.ipcMain.listeners.get(IPC_CHANNELS.overlayPlaybackStop)(null, {
    jobId: 'guest-stop',
  });

  assert.equal(harness.socket.emitted.length, 0);
  assert.equal(harness.pendingStatePayloads.length, 0);
  assert.equal(harness.pendingStopPayloads.length, 0);
});
