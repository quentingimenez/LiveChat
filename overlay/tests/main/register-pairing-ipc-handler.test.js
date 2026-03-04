const test = require('node:test');
const assert = require('node:assert/strict');

const { registerPairingIpcHandler } = require('../../dist/main/ipc/register-pairing-ipc-handler.js');
const { IPC_CHANNELS } = require('../../dist/shared/ipc.js');

function createIpcMainMock() {
  const handles = new Map();

  return {
    handles,
    handle(channel, handler) {
      handles.set(channel, handler);
    },
  };
}

test('register-pairing-ipc-handler wires pairing consume channel and runs side effects', async () => {
  const ipcMain = createIpcMainMock();
  const saveConfigCalls = [];
  const requestCalls = [];
  const setGuestModeCalls = [];
  let settingsCalls = 0;
  let createOverlayWindowCalls = 0;
  let connectOverlaySocketCalls = 0;
  let closePairingWindowCalls = 0;
  let updateTrayMenuCalls = 0;

  registerPairingIpcHandler({
    logger: { info() {}, warn() {}, error() {} },
    ipcMain,
    loadConfig: () => ({}),
    saveConfig: (nextValues) => {
      saveConfigCalls.push(nextValues);
      return nextValues;
    },
    hasPairingConfig: () => true,
    isGuestModeEnabled: () => false,
    normalizeMemeBindings: () => ({}),
    normalizeServerUrl: (serverUrl) => `${serverUrl || ''}`.trim().replace(/\/+$/, ''),
    httpRequestJson: async (url, payload, options = {}) => {
      requestCalls.push({ url, payload, options });
      return {
        statusCode: 200,
        body: JSON.stringify({
          apiBaseUrl: 'https://api.livechat.test/',
          clientToken: 'token-1',
          clientId: 'client-1',
          guildId: 'guild-1',
          authorName: 'Author',
          deviceName: 'Device X',
          sessionMode: 'normal',
        }),
      };
    },
    isLikelyTlsError: () => false,
    formatNetworkError: () => 'network_error',
    setGuestMode: (checked) => {
      setGuestModeCalls.push(checked);
      return {
        enabled: true,
        serverUrl: 'https://api.livechat.test',
        clientToken: 'token-1',
        guildId: 'guild-1',
      };
    },
    sendOverlaySettingsToRenderer: () => {
      settingsCalls += 1;
    },
    createOverlayWindow: () => {
      createOverlayWindowCalls += 1;
    },
    connectOverlaySocket: () => {
      connectOverlaySocketCalls += 1;
    },
    closePairingWindow: () => {
      closePairingWindowCalls += 1;
    },
    updateTrayMenu: () => {
      updateTrayMenuCalls += 1;
    },
  });

  assert.equal(ipcMain.handles.has(IPC_CHANNELS.pairingConsume), true);

  const response = await ipcMain.handles.get(IPC_CHANNELS.pairingConsume)(null, {
    serverUrl: 'https://bot.livechat.test/',
    code: ' abcd ',
    deviceName: ' Request Device ',
  });

  assert.deepEqual(response, { ok: true });
  assert.equal(requestCalls.length, 1);
  assert.equal(requestCalls[0].url, 'https://bot.livechat.test/overlay/pair/consume');
  assert.deepEqual(requestCalls[0].payload, {
    code: 'ABCD',
    deviceName: 'Request Device',
  });
  assert.equal(requestCalls[0].options.rejectUnauthorized, true);

  assert.equal(saveConfigCalls.length, 1);
  assert.deepEqual(saveConfigCalls[0], {
    serverUrl: 'https://api.livechat.test',
    clientToken: 'token-1',
    clientId: 'client-1',
    guildId: 'guild-1',
    authorName: 'Author',
    deviceName: 'Device X',
    guestMode: false,
  });

  assert.deepEqual(setGuestModeCalls, [false]);
  assert.equal(settingsCalls, 1);
  assert.equal(createOverlayWindowCalls, 1);
  assert.equal(connectOverlaySocketCalls, 1);
  assert.equal(closePairingWindowCalls, 1);
  assert.equal(updateTrayMenuCalls, 1);
});

test('register-pairing-ipc-handler propagates service validation errors', async () => {
  const ipcMain = createIpcMainMock();

  registerPairingIpcHandler({
    logger: { info() {}, warn() {}, error() {} },
    ipcMain,
    loadConfig: () => ({}),
    saveConfig: () => ({}),
    hasPairingConfig: () => false,
    isGuestModeEnabled: () => false,
    normalizeMemeBindings: () => ({}),
    normalizeServerUrl: (serverUrl) => `${serverUrl || ''}`.trim().replace(/\/+$/, ''),
    httpRequestJson: async () => ({ statusCode: 200, body: '{}' }),
    isLikelyTlsError: () => false,
    formatNetworkError: () => 'network_error',
    setGuestMode: () => ({ enabled: false }),
    sendOverlaySettingsToRenderer: () => {},
    createOverlayWindow: () => {},
    connectOverlaySocket: () => {},
    closePairingWindow: () => {},
    updateTrayMenu: () => {},
  });

  await assert.rejects(() => ipcMain.handles.get(IPC_CHANNELS.pairingConsume)(null, {}), /missing_required_fields/);
});
