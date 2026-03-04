const test = require('node:test');
const assert = require('node:assert/strict');

const { createOverlayPeersService } = require('../../dist/main/services/overlay-peers-service.js');

test('overlay-peers-service normalizes peers and builds tooltip summary', () => {
  const service = createOverlayPeersService({
    maxOtherActiveOverlaysInTooltip: 2
  });

  service.setConnectedOverlayPeers([
    { clientId: 'self', label: 'Overlay-Main' },
    { clientId: 'b', label: 'Overlay-Beta' },
    { clientId: 'c', label: 'Overlay-Charlie' },
    { clientId: 'b', label: 'Duplicate' },
    { clientId: '', label: 'Invalid' },
    null
  ]);

  const tooltip = service.buildTrayTooltip({
    config: {
      clientId: 'self',
      authorName: null
    },
    connectionStateLabel: 'Connecté'
  });

  assert.match(tooltip, /^Overlay Connecté/);
  assert.match(tooltip, /Autres: 2/);
  assert.match(tooltip, /Beta/);
  assert.match(tooltip, /Charlie/);
});

test('overlay-peers-service prefers authorName in status suffix', () => {
  const service = createOverlayPeersService({
    maxOtherActiveOverlaysInTooltip: 1
  });

  service.setConnectedOverlayPeers([{ clientId: 'self', label: 'Overlay-Device' }]);

  const tooltip = service.buildTrayTooltip({
    config: {
      clientId: 'self',
      authorName: 'Streamer'
    },
    connectionStateLabel: 'Connexion...'
  });

  assert.match(tooltip, /Overlay Connexion\.\.\. \(Streamer\)/);
});
