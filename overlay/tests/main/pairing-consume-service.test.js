const test = require('node:test');
const assert = require('node:assert/strict');

const {
  createPairingConsumeService,
  parsePairingConsumeResponse,
} = require('../../dist/main/services/pairing-consume-service.js');

function createHarness(overrides = {}) {
  const requestCalls = [];
  const saveConfigCalls = [];
  const setGuestModeCalls = [];
  let sendOverlaySettingsCalls = 0;
  let createOverlayWindowCalls = 0;
  let connectOverlaySocketCalls = 0;
  let closePairingWindowCalls = 0;
  let updateTrayMenuCalls = 0;

  const defaultHttpRequestJson = async (url, payload, options = {}) => {
    requestCalls.push({ url, payload, options });

    return {
      statusCode: 200,
      body: JSON.stringify({
        apiBaseUrl: 'https://api.livechat.test/',
        clientToken: 'token-1',
        clientId: 'client-1',
        guildId: 'guild-1',
        authorName: 'Author',
        deviceName: 'Resolved Device',
        sessionMode: 'invite_read_only',
      }),
    };
  };

  const service = createPairingConsumeService({
    saveConfig(nextValues) {
      saveConfigCalls.push(nextValues);
      return nextValues;
    },
    hasPairingConfig(config) {
      if (typeof overrides.hasPairingConfig === 'function') {
        return overrides.hasPairingConfig(config);
      }

      return Boolean(config && config.serverUrl && config.clientToken && config.guildId);
    },
    normalizeServerUrl(serverUrl) {
      if (typeof overrides.normalizeServerUrl === 'function') {
        return overrides.normalizeServerUrl(serverUrl);
      }

      return `${serverUrl || ''}`.trim().replace(/\/+$/, '');
    },
    httpRequestJson: overrides.httpRequestJson || defaultHttpRequestJson,
    isLikelyTlsError: overrides.isLikelyTlsError || (() => false),
    formatNetworkError: overrides.formatNetworkError || ((error, endpoint) => `network_error (${endpoint}): ${error}`),
    setGuestMode(checked) {
      setGuestModeCalls.push(checked);

      if (typeof overrides.setGuestMode === 'function') {
        return overrides.setGuestMode(checked);
      }

      return {
        enabled: true,
        serverUrl: 'https://api.livechat.test',
        clientToken: 'token-1',
        guildId: 'guild-1',
      };
    },
    sendOverlaySettingsToRenderer() {
      sendOverlaySettingsCalls += 1;
    },
    createOverlayWindow() {
      createOverlayWindowCalls += 1;
    },
    connectOverlaySocket() {
      connectOverlaySocketCalls += 1;
    },
    closePairingWindow() {
      closePairingWindowCalls += 1;
    },
    updateTrayMenu() {
      updateTrayMenuCalls += 1;
    },
  });

  return {
    service,
    requestCalls,
    saveConfigCalls,
    setGuestModeCalls,
    getCounts() {
      return {
        sendOverlaySettingsCalls,
        createOverlayWindowCalls,
        connectOverlaySocketCalls,
        closePairingWindowCalls,
        updateTrayMenuCalls,
      };
    },
  };
}

test('pairing-consume-service parses pairing response payload', () => {
  assert.deepEqual(parsePairingConsumeResponse('{"ok":true}'), { ok: true });
  assert.throws(() => parsePairingConsumeResponse('not-json'), /invalid_pairing_response/);
  assert.throws(() => parsePairingConsumeResponse('[]'), /invalid_pairing_response/);
});

test('pairing-consume-service rejects when required fields are missing', async () => {
  const { service } = createHarness();

  await assert.rejects(() => service.consumePairing({ serverUrl: '', code: '' }), /missing_required_fields/);
  await assert.rejects(() => service.consumePairing({ serverUrl: 'https://api.livechat.test' }), /missing_required_fields/);
});

test('pairing-consume-service uses formatted network errors for non-TLS failures', async () => {
  const { service } = createHarness({
    httpRequestJson: async () => {
      throw new Error('connection_refused');
    },
    isLikelyTlsError: () => false,
    formatNetworkError: (_error, endpoint) => `network_error (${endpoint})`,
  });

  await assert.rejects(
    () => service.consumePairing({ serverUrl: 'https://bot.livechat.test', code: 'ABCD' }),
    /network_error \(https:\/\/bot\.livechat\.test\/overlay\/pair\/consume\)/
  );
});

test('pairing-consume-service retries with insecure TLS for likely certificate errors', async () => {
  const requestCalls = [];

  const { service } = createHarness({
    httpRequestJson: async (url, payload, options = {}) => {
      requestCalls.push({ url, payload, options });

      if (requestCalls.length === 1) {
        throw new Error('self_signed_cert');
      }

      return {
        statusCode: 200,
        body: JSON.stringify({
          apiBaseUrl: 'https://api.livechat.test/',
          clientToken: 'token-1',
          clientId: 'client-1',
          guildId: 'guild-1',
          sessionMode: 'normal',
        }),
      };
    },
    isLikelyTlsError: () => true,
  });

  const result = await service.consumePairing({ serverUrl: 'https://bot.livechat.test/', code: ' A1B2 ' });

  assert.deepEqual(result, { ok: true });
  assert.equal(requestCalls.length, 2);
  assert.equal(requestCalls[0].options.rejectUnauthorized, true);
  assert.equal(requestCalls[1].options.rejectUnauthorized, false);
  assert.equal(requestCalls[0].payload.code, 'A1B2');
});

test('pairing-consume-service persists pairing and opens overlay when pairing config is complete', async () => {
  const harness = createHarness();

  const result = await harness.service.consumePairing({
    serverUrl: 'https://bot.livechat.test/',
    code: ' x9y8 ',
    deviceName: ' Requested Device ',
  });

  assert.deepEqual(result, { ok: true });
  assert.equal(harness.requestCalls.length, 1);
  assert.equal(harness.saveConfigCalls.length, 1);
  assert.deepEqual(harness.saveConfigCalls[0], {
    serverUrl: 'https://api.livechat.test',
    clientToken: 'token-1',
    clientId: 'client-1',
    guildId: 'guild-1',
    authorName: 'Author',
    deviceName: 'Resolved Device',
    guestMode: true,
  });
  assert.deepEqual(harness.setGuestModeCalls, [true]);

  const counts = harness.getCounts();
  assert.equal(counts.sendOverlaySettingsCalls, 1);
  assert.equal(counts.createOverlayWindowCalls, 1);
  assert.equal(counts.connectOverlaySocketCalls, 1);
  assert.equal(counts.closePairingWindowCalls, 1);
  assert.equal(counts.updateTrayMenuCalls, 1);
});

test('pairing-consume-service does not open overlay when resulting config is disabled', async () => {
  const harness = createHarness({
    setGuestMode: () => ({
      enabled: false,
      serverUrl: 'https://api.livechat.test',
      clientToken: 'token-1',
      guildId: 'guild-1',
    }),
  });

  await harness.service.consumePairing({
    serverUrl: 'https://bot.livechat.test',
    code: 'QWER',
  });

  const counts = harness.getCounts();
  assert.equal(counts.createOverlayWindowCalls, 0);
  assert.equal(counts.connectOverlaySocketCalls, 0);
  assert.equal(counts.closePairingWindowCalls, 1);
  assert.equal(counts.updateTrayMenuCalls, 1);
});
