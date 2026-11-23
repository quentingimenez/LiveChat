// Unit tests of the stop on this device only: what the overlay renderer
// reports to the shell when it obeys an `overlay://stop`.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyPlaybackSessionUtils } from '../../web/dist/renderer/overlay/legacy-playback-session-utils.js';
import { createOverlayLegacyPlaybackUtils } from '../../web/dist/renderer/overlay/legacy-playback-utils.js';
import { createOverlayLegacyOverlayResetUtils } from '../../web/dist/renderer/overlay/legacy-overlay-reset-utils.js';
import { createOverlayLegacyRuntimePlaybackCountdownHandlersUtils } from '../../web/dist/renderer/overlay/legacy-runtime-playback-countdown-handlers-utils.js';

const playbackUtils = createOverlayLegacyPlaybackUtils();
const playbackSessionUtils = createOverlayLegacyPlaybackSessionUtils();

// The real handlers, with the DOM-facing collaborators stubbed.
function handlers(options = {}) {
  const reports = [];
  const state = {
    resetTimer: null,
    countdownState: { timer: null, remainingMs: null, lastTickAtMs: null, paused: false, autoClear: false },
    playbackSyncTimer: null,
    activePlayableElement: null,
    activePlayback: null,
    activeObjectUrl: null,
  };
  const created = createOverlayLegacyRuntimePlaybackCountdownHandlersUtils().createRuntimePlaybackCountdownHandlers({
    nodes: options.nodes || {},
    state,
    utils: {},
    countdownUtils: { getRemainingCountdownMs: () => null, clearCountdown: () => {} },
    playbackUtils,
    playbackSessionUtils,
    overlayResetUtils: options.overlayResetUtils || { releaseObjectUrl: () => null, resetOverlayLayers: () => {} },
    clearMediaHeaderLayoutRaf: () => {},
    resetMediaFrameState: () => {},
    scheduleMediaHeaderLayout: () => {},
    reportPlaybackState: () => {},
    reportPlaybackStop: (payload) => reports.push(payload),
    setTimeout: () => 1,
    clearTimeout: () => {},
    setInterval: () => 1,
    clearInterval: () => {},
    nowMs: () => 0,
    logDebug: () => {},
  });
  return { ...created, reports, state };
}

test('only a local stop is told apart in the report reason', () => {
  assert.equal(playbackUtils.stopReportReason({ reason: 'local', jobId: 'job-1' }), 'local');
  for (const payload of [{ reason: 'manual' }, { reason: 'preempt' }, { reason: 'disabled' }, {}, null, undefined]) {
    assert.equal(playbackUtils.stopReportReason(payload), null);
  }
});

test('a local stop clears the playback and reports it as local', () => {
  const { startPlaybackSession, clearOverlay, reports, state } = handlers();
  startPlaybackSession('job-1');
  clearOverlay({ reason: playbackUtils.stopReportReason({ reason: 'local', jobId: 'job-1' }) });
  assert.deepEqual(reports, [{ jobId: 'job-1', reason: 'local' }]);
  assert.equal(state.activePlayback, null);
  // Nothing left to report: a second stop stays silent.
  clearOverlay({ reason: 'local' });
  assert.equal(reports.length, 1);
});

test('every other end of playback is reported without a reason', () => {
  const { startPlaybackSession, clearOverlay, reports } = handlers();
  startPlaybackSession('job-1');
  clearOverlay();
  startPlaybackSession('job-2');
  clearOverlay({ reason: playbackUtils.stopReportReason({ reason: 'manual' }) });
  assert.deepEqual(reports, [{ jobId: 'job-1' }, { jobId: 'job-2' }]);
});

test('a stop pauses and releases the media element before clearing the view', () => {
  const calls = [];
  const element = {
    pause: () => calls.push('pause'),
    removeAttribute: (name) => calls.push(`remove:${name}`),
    load: () => calls.push('load'),
  };
  const resetUtils = createOverlayLegacyOverlayResetUtils();
  const { startPlaybackSession, clearOverlay, state } = handlers({
    overlayResetUtils: {
      releaseMediaElement: resetUtils.releaseMediaElement,
      releaseMediaElements: resetUtils.releaseMediaElements,
      releaseObjectUrl: () => null,
      resetOverlayLayers: () => {},
    },
  });
  state.activePlayableElement = element;
  startPlaybackSession('job-1');

  clearOverlay();

  assert.deepEqual(calls, ['pause', 'remove:src', 'load']);
  assert.equal(state.activePlayableElement, null);
});
