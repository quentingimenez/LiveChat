const test = require('node:test');
const assert = require('node:assert/strict');

const { createOverlayConfigIpcService } = require('../../dist/main/services/overlay-config-ipc-service.js');

test('overlay-config-ipc-service returns renderer config payload with normalized bindings', () => {
  const service = createOverlayConfigIpcService({
    loadConfig: () => ({
      serverUrl: 'https://api.livechat.test',
      clientToken: 'token-1',
      guildId: 'guild-1',
      clientId: 'client-1',
      showText: true,
      volume: 0.66,
      memeBindings: {
        'Ctrl+1': 'item-1',
        '': 'invalid',
      },
    }),
    isGuestModeEnabled: () => false,
    normalizeMemeBindings: () => ({
      'Ctrl+1': 'item-1',
    }),
  });

  assert.deepEqual(service.getConfig(), {
    serverUrl: 'https://api.livechat.test',
    clientToken: 'token-1',
    guildId: 'guild-1',
    clientId: 'client-1',
    guestMode: false,
    showText: true,
    volume: 0.66,
    memeBindings: {
      'Ctrl+1': 'item-1',
    },
  });
});
