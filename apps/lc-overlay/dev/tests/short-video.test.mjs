// A video is shown for exactly the duration of its own media: the overlay
// clears as soon as the element ends, whatever the duration of the job. A
// one-second clip (an animated GIF converted to MP4, typically) must not be
// looped, held or otherwise stretched to fill a longer job.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyMediaRenderUtils } from '../../web/dist/renderer/overlay/legacy-media-render-utils.js';

const { renderMedia } = createOverlayLegacyMediaRenderUtils();

// Minimal DOM stand-ins for the renderer's `instanceof HTMLElement` checks.
class FakeElement {
  constructor() {
    this.listeners = {};
    this.currentTime = 0;
    this.playCalls = 0;
  }
  addEventListener(name, handler) {
    (this.listeners[name] ||= []).push(handler);
  }
  dispatch(name) {
    for (const handler of this.listeners[name] || []) {
      handler();
    }
  }
  play() {
    this.playCalls += 1;
    return Promise.resolve();
  }
}
globalThis.HTMLElement = FakeElement;

const video = (durationSec, extra = {}) => ({ kind: 'video', url: 'https://media.test/clip', durationSec, ...extra });

// Runs the real `renderMedia` on a fake video and returns what happened.
async function play(payload) {
  const element = new FakeElement();
  const calls = [];
  await renderMedia({
    payload,
    buildAuthorizedMediaUrl: (url) => url,
    setActiveObjectUrl: () => {},
    createMediaElement: () => element,
    attachMediaDiagnostics: () => {},
    ensureMediaFrame: () => new FakeElement(),
    mediaLayer: new FakeElement(),
    attachMediaElement: () => true,
    scheduleMediaHeaderLayout: () => {},
    applyVolume: () => {},
    setActivePlayableElement: () => {},
    applyMediaStartOffset: () => {},
    resolveStartOffsetSec: () => null,
    resumeCountdown: () => {},
    pauseCountdown: () => {},
    setPlaybackState: () => {},
    clearOverlay: () => calls.push('clearOverlay'),
    logAutoplayError: () => {},
  });
  return { element, calls };
}

test('a one-second clip leaves the screen at its end (regression: it was looped for three seconds)', async () => {
  const { element, calls } = await play({ jobId: 'job-1', durationSec: 3, media: video(1) });
  assert.equal(element.playCalls, 1);

  element.currentTime = 1;
  element.dispatch('ended');
  assert.deepEqual(calls, ['clearOverlay']);
  assert.equal(element.playCalls, 1, 'the clip must not be replayed');
  assert.equal(element.currentTime, 1, 'the clip must not be rewound');
});

test('a video lasting as long as its job still clears at its end', async () => {
  const { element, calls } = await play({ jobId: 'job-1', durationSec: 12, media: video(12) });
  element.dispatch('ended');
  assert.deepEqual(calls, ['clearOverlay']);
  assert.equal(element.playCalls, 1);
});

test('a clip resumed after a preemption clears at its own end', async () => {
  const { element, calls } = await play({
    jobId: 'job-1',
    durationSec: 2,
    media: video(1, { startOffsetSec: 0.5 }),
  });
  element.dispatch('ended');
  assert.deepEqual(calls, ['clearOverlay']);
  assert.equal(element.playCalls, 1);
});
