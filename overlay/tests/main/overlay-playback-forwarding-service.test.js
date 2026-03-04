const test = require('node:test');
const assert = require('node:assert/strict');

const {
  createOverlayPlaybackForwardingService,
  normalizePlaybackStopPayload,
} = require('../../dist/main/services/overlay-playback-forwarding-service.js');
const { OVERLAY_SOCKET_EVENTS } = require('../../dist/shared/socket-events.js');

function createLoggerMock() {
  const logs = {
    info: [],
    warn: [],
    error: [],
  };

  return {
    logs,
    logger: {
      info(message) {
        logs.info.push(message);
      },
      warn(message) {
        logs.warn.push(message);
      },
      error(message) {
        logs.error.push(message);
      },
    },
  };
}

function createSocketMock(connected = true) {
  const emitted = [];

  return {
    connected,
    emitted,
    emit(event, payload) {
      emitted.push({ event, payload });
    },
  };
}

test('overlay-playback-forwarding-service normalizes playback-stop payload', () => {
  assert.deepEqual(normalizePlaybackStopPayload({ jobId: '  abc  ' }), { jobId: 'abc' });
  assert.deepEqual(normalizePlaybackStopPayload({ jobId: '' }), { jobId: 'unknown' });
  assert.deepEqual(normalizePlaybackStopPayload(null), { jobId: 'unknown' });
});

test('overlay-playback-forwarding-service forwards overlay errors only when socket is connected', () => {
  const { logger } = createLoggerMock();
  const socket = createSocketMock(true);
  let guestModeEnabled = false;

  const service = createOverlayPlaybackForwardingService({
    logger,
    isGuestModeEnabled: () => guestModeEnabled,
    getOverlaySocket: () => socket,
    setPendingPlaybackStatePayload() {},
    setPendingPlaybackStopPayload() {},
  });

  service.handleOverlayError({ code: 'E_FAIL' });
  assert.deepEqual(socket.emitted[0], {
    event: OVERLAY_SOCKET_EVENTS.ERROR,
    payload: { code: 'E_FAIL' },
  });

  guestModeEnabled = true;
  service.handleOverlayError({ code: 'E_GUEST' });
  assert.equal(socket.emitted.length, 1);

  guestModeEnabled = false;
  socket.connected = false;
  service.handleOverlayError({ code: 'E_OFFLINE' });
  assert.equal(socket.emitted.length, 1);
});

test('overlay-playback-forwarding-service buffers playback-state while socket is offline', () => {
  const { logger, logs } = createLoggerMock();
  const socket = createSocketMock(false);
  const pendingStatePayloads = [];

  const service = createOverlayPlaybackForwardingService({
    logger,
    isGuestModeEnabled: () => false,
    getOverlaySocket: () => socket,
    setPendingPlaybackStatePayload(payload) {
      pendingStatePayloads.push(payload);
    },
    setPendingPlaybackStopPayload() {},
  });

  service.handleOverlayPlaybackState({
    jobId: '  job-42 ',
    state: ' playing ',
  });
  service.handleOverlayPlaybackState(undefined);

  assert.equal(socket.emitted.length, 0);
  assert.deepEqual(pendingStatePayloads, [
    {
      jobId: '  job-42 ',
      state: ' playing ',
    },
    null,
  ]);
  assert.equal(logs.warn.length, 2);
  assert.match(logs.warn[0], /job-42/);
  assert.match(logs.warn[0], /playing/);
});

test('overlay-playback-forwarding-service forwards playback-state when socket is connected', () => {
  const { logger, logs } = createLoggerMock();
  const socket = createSocketMock(true);

  const service = createOverlayPlaybackForwardingService({
    logger,
    isGuestModeEnabled: () => false,
    getOverlaySocket: () => socket,
    setPendingPlaybackStatePayload() {},
    setPendingPlaybackStopPayload() {},
  });

  const payload = {
    jobId: 'job-7',
    state: 'playing',
    remainingMs: 1800,
  };

  service.handleOverlayPlaybackState(payload);

  assert.deepEqual(socket.emitted, [
    {
      event: OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE,
      payload,
    },
  ]);
  assert.equal(logs.info.length, 1);
  assert.match(logs.info[0], /remainingMs: 1800/);
});

test('overlay-playback-forwarding-service handles playback-stop offline and online', () => {
  const { logger, logs } = createLoggerMock();
  const socket = createSocketMock(false);
  const pendingStopPayloads = [];

  const service = createOverlayPlaybackForwardingService({
    logger,
    isGuestModeEnabled: () => false,
    getOverlaySocket: () => socket,
    setPendingPlaybackStatePayload() {},
    setPendingPlaybackStopPayload(payload) {
      pendingStopPayloads.push(payload);
    },
  });

  service.handleOverlayPlaybackStop({ jobId: '  stop-1 ' });
  assert.equal(socket.emitted.length, 0);
  assert.deepEqual(pendingStopPayloads[0], { jobId: 'stop-1' });
  assert.match(logs.warn[0], /stop-1/);

  socket.connected = true;
  service.handleOverlayPlaybackStop({ jobId: '  stop-2 ' });
  service.handleOverlayPlaybackStop({});

  assert.deepEqual(socket.emitted, [
    {
      event: OVERLAY_SOCKET_EVENTS.STOP,
      payload: { jobId: 'stop-2' },
    },
    {
      event: OVERLAY_SOCKET_EVENTS.STOP,
      payload: { jobId: 'unknown' },
    },
  ]);
  assert.equal(logs.info.length, 2);
});
