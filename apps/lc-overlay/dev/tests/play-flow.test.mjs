// Unit tests of the overlay play flow: countdown and auto-clear rules.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyPlayFlowUtils } from '../../web/dist/renderer/overlay/legacy-play-flow-utils.js';

const { runPlayFlow } = createOverlayLegacyPlayFlowUtils();

function recorder(payload) {
  const calls = [];
  const record = (name) => (...args) => {
    calls.push([name, ...args]);
  };
  return {
    calls,
    params: {
      payload,
      clearOverlay: record('clearOverlay'),
      startPlaybackSession: record('startPlaybackSession'),
      applyOverlayInfo: record('applyOverlayInfo'),
      renderTweetCard: () => false,
      renderMedia: async () => {},
      applyMediaHeaderAuthor: record('applyMediaHeaderAuthor'),
      applyMediaFooterText: record('applyMediaFooterText'),
      logRenderError: record('logRenderError'),
      reportError: record('reportError'),
      startCountdown: record('startCountdown'),
      pauseCountdown: record('pauseCountdown'),
      setResetTimer: record('setResetTimer'),
      setTimeout: (callback, delayMs) => {
        calls.push(['setTimeout', delayMs]);
        return 1;
      },
      clearCountdown: record('clearCountdown'),
    },
  };
}

const image = { kind: 'image', url: 'assets/test-media.svg' };

test('an image clears itself when its countdown ends', async () => {
  const { calls, params } = recorder({ durationSec: 5, media: image });
  await runPlayFlow(params);
  assert.deepEqual(
    calls.find(([name]) => name === 'startCountdown'),
    ['startCountdown', 5, { autoClear: true }],
  );
  assert.deepEqual(
    calls.find(([name]) => name === 'setTimeout'),
    ['setTimeout', 5100],
  );
  assert.equal(calls.some(([name]) => name === 'pauseCountdown'), false);
});

test('a held card shows a frozen countdown and never clears itself', async () => {
  const { calls, params } = recorder({ durationSec: 10, hold: true, test: true, media: image });
  await runPlayFlow(params);
  const names = calls.map(([name]) => name);
  assert.deepEqual(
    calls.find(([name]) => name === 'startCountdown'),
    ['startCountdown', 10, { autoClear: false }],
  );
  assert.ok(names.indexOf('pauseCountdown') > names.indexOf('startCountdown'));
  assert.equal(names.includes('setTimeout'), false);
  assert.equal(names.includes('setResetTimer'), false);
});
