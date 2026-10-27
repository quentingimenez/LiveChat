const test = require('node:test');
const assert = require('node:assert/strict');

const {
  getPayloadField,
  registerOverlaySocketListeners,
  toTrimmedString,
} = require('../../dist/main/services/socket-listeners-service.js');

function createSocketMock() {
  const listeners = new Map();
  const ioListeners = new Map();

  return {
    listeners,
    ioListeners,
    on(event, listener) {
      listeners.set(event, listener);
    },
    io: {
      on(event, listener) {
        ioListeners.set(event, listener);
      },
    },
  };
}

test('socket-listeners-service trims payload fields safely', () => {
  assert.equal(toTrimmedString('  abc  '), 'abc');
  assert.equal(toTrimmedString(null), '');

  assert.equal(getPayloadField({ message: '  failed  ' }, 'message'), 'failed');
  assert.equal(getPayloadField({ message: 123 }, 'message'), '');
  assert.equal(getPayloadField(null, 'message'), '');
});

test('socket-listeners-service wires listeners and state transitions', () => {
  const socket = createSocketMock();
  const states = [];
  const peersPayloads = [];
  const playPayloads = [];
  const stopPayloads = [];
  let heartbeatStarted = 0;
  let heartbeatStopped = 0;
  let heartbeatEmitted = 0;
  let pendingFlushed = 0;
  const logErrors = [];

  let currentConfig = {
    enabled: true,
    guildId: 'guild-1',
    clientId: 'client-1',
    clientToken: 'token',
  };

  registerOverlaySocketListeners({
    overlaySocket: socket,
    cfg: currentConfig,
    logger: {
      info() {},
      warn() {},
      error(...args) {
        logErrors.push(args.join(' '));
      },
    },
    loadConfig: () => currentConfig,
    hasPairingConfig: (config) => Boolean(config && config.clientToken && config.guildId),
    sendOverlayPlay: (payload) => {
      playPayloads.push(payload);
    },
    sendOverlayStop: (payload) => {
      stopPayloads.push(payload);
    },
    onConnectedPeersChange: (peers) => {
      peersPayloads.push(peers);
    },
    onConnectionStateChange: (state, reason = '') => {
      states.push({ state, reason });
    },
    startHeartbeatLoop: () => {
      heartbeatStarted += 1;
    },
    stopHeartbeatLoop: () => {
      heartbeatStopped += 1;
    },
    emitHeartbeat: () => {
      heartbeatEmitted += 1;
    },
    flushPendingPayloads: () => {
      pendingFlushed += 1;
    },
  });

  assert.equal(typeof socket.listeners.get('connect'), 'function');
  assert.equal(typeof socket.listeners.get('disconnect'), 'function');
  assert.equal(typeof socket.listeners.get('connect_error'), 'function');
  assert.equal(typeof socket.ioListeners.get('reconnect_attempt'), 'function');

  socket.listeners.get('connect')();
  assert.equal(heartbeatStarted, 1);
  assert.equal(heartbeatEmitted, 1);
  assert.equal(pendingFlushed, 1);
  assert.deepEqual(states[0], { state: 'connected', reason: '' });

  socket.listeners.get('overlay:play')({ kind: 'play' });
  socket.listeners.get('overlay:stop')({ kind: 'stop' });
  assert.deepEqual(playPayloads, [{ kind: 'play' }]);
  assert.deepEqual(stopPayloads, [{ kind: 'stop' }]);

  socket.listeners.get('overlay:peers')({ guildId: 'other-guild', peers: [{ id: 'x' }] });
  assert.equal(peersPayloads.length, 0);

  socket.listeners.get('overlay:peers')({ guildId: 'guild-1', peers: [{ id: 'p1' }] });
  assert.deepEqual(peersPayloads[0], [{ id: 'p1' }]);

  socket.listeners.get('disconnect')('io client disconnect');
  assert.equal(heartbeatStopped, 1);
  assert.deepEqual(states[1], { state: 'disconnected', reason: 'io client disconnect' });

  currentConfig = {
    enabled: false,
    guildId: 'guild-1',
    clientId: 'client-1',
    clientToken: 'token',
  };
  socket.listeners.get('disconnect')('network reset');
  assert.deepEqual(states[2], { state: 'disabled', reason: 'network reset' });

  currentConfig = {
    enabled: true,
    guildId: '',
    clientId: 'client-1',
    clientToken: '',
  };
  socket.listeners.get('disconnect')('network reset');
  assert.deepEqual(states[3], { state: 'not_paired', reason: 'network reset' });

  socket.listeners.get('connect_error')({ message: '  refused  ' });
  assert.deepEqual(states[4], { state: 'error', reason: 'refused' });
  assert.equal(logErrors.length, 1);

  socket.ioListeners.get('reconnect_attempt')();
  assert.deepEqual(states[5], { state: 'reconnecting', reason: '' });
});
