// Unit tests of the extension of a video whose job outlasts it (a one-second
// GIF converted to MP4 gets a minimum display time from the server): the
// overlay replays a silent clip, or holds the last frame of one with sound,
// until the job's countdown ends.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyMediaRenderUtils } from '../../web/dist/renderer/overlay/legacy-media-render-utils.js';
import { createOverlayLegacyPlaybackUtils } from '../../web/dist/renderer/overlay/legacy-playback-utils.js';

const { videoExtensionSec, resolveVideoEndAction } = createOverlayLegacyPlaybackUtils();
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

test('a job outlasting its video leaves time to fill', () => {
  // The server's minimum display time: 3 s for a 1 s clip.
  assert.equal(videoExtensionSec({ durationSec: 3, media: video(1) }), 2);
  // A longer duration chosen by the sender.
  assert.equal(videoExtensionSec({ durationSec: 10, media: video(8) }), 2);
  // Resumed after a preemption: only the rest of the clip counts.
  assert.equal(videoExtensionSec({ durationSec: 3, media: video(5, { startOffsetSec: 4 }) }), 2);
});

test('nothing is extended when the job does not outlast a known video', () => {
  assert.equal(videoExtensionSec({ durationSec: 12, media: video(12) }), 0);
  assert.equal(videoExtensionSec({ durationSec: 2, media: video(12) }), 0);
  // Unknown media duration: the job duration is only a fallback.
  assert.equal(videoExtensionSec({ durationSec: 30, media: video(null) }), 0);
  assert.equal(videoExtensionSec({ durationSec: 30, media: { kind: 'video', url: 'x' } }), 0);
  assert.equal(videoExtensionSec({ durationSec: 30, media: { kind: 'audio', url: 'x', durationSec: 1 } }), 0);
  assert.equal(videoExtensionSec({ durationSec: 5, media: { kind: 'image', url: 'x' } }), 0);
  assert.equal(videoExtensionSec({ durationSec: 5 }), 0);
  assert.equal(videoExtensionSec(null), 0);
});

test('an ended video is replayed, held or cleared', () => {
  assert.equal(resolveVideoEndAction({ extend: true, remainingMs: 2000, hasAudio: false }), 'replay');
  assert.equal(resolveVideoEndAction({ extend: true, remainingMs: 2000, hasAudio: true }), 'hold');
  // Job over (or about to be): leave the screen.
  assert.equal(resolveVideoEndAction({ extend: true, remainingMs: 100, hasAudio: false }), 'clear');
  assert.equal(resolveVideoEndAction({ extend: true, remainingMs: 0, hasAudio: true }), 'clear');
  assert.equal(resolveVideoEndAction({ extend: true, remainingMs: null, hasAudio: false }), 'clear');
  // A video playing for its own duration ends the playback, as before.
  assert.equal(resolveVideoEndAction({ extend: false, remainingMs: 20000, hasAudio: false }), 'clear');
});

// Runs the real `renderMedia` on a fake video and returns what happened.
async function play(payload, { hasAudio = false } = {}) {
  const element = new FakeElement();
  const calls = [];
  const clock = { remainingMs: null };
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
    videoExtensionSec,
    resolveVideoEndAction,
    videoHasAudioTrack: () => hasAudio,
    getRemainingCountdownMs: () => clock.remainingMs,
    setCountdownAutoClear: (value) => calls.push(`autoClear:${value}`),
    logAutoplayError: () => {},
  });
  // The play flow starts the countdown once `renderMedia` has returned.
  clock.remainingMs = payload.durationSec * 1000;
  return { element, calls, clock };
}

test('a one-second silent clip loops until its job ends (regression: it vanished after one second)', async () => {
  const { element, calls, clock } = await play({ jobId: 'job-1', durationSec: 3, media: video(1) });
  assert.equal(element.playCalls, 1);

  clock.remainingMs = 2000;
  element.currentTime = 1;
  element.dispatch('ended');
  assert.deepEqual(calls, ['autoClear:true']);
  assert.equal(element.currentTime, 0);
  assert.equal(element.playCalls, 2);

  clock.remainingMs = 1000;
  element.dispatch('ended');
  assert.equal(element.playCalls, 3);
  assert.equal(calls.includes('clearOverlay'), false);

  // Third pass ends with the job: the overlay is cleared.
  clock.remainingMs = 0;
  element.dispatch('ended');
  assert.equal(element.playCalls, 3);
  assert.equal(calls.at(-1), 'clearOverlay');
});

test('a short clip with sound plays once and holds its last frame', async () => {
  const { element, calls, clock } = await play({ jobId: 'job-1', durationSec: 3, media: video(1) }, { hasAudio: true });
  clock.remainingMs = 2000;
  element.currentTime = 1;
  element.dispatch('ended');
  // Not replayed, not cleared: the countdown clears the overlay at zero.
  assert.deepEqual(calls, ['autoClear:true']);
  assert.equal(element.playCalls, 1);
  assert.equal(element.currentTime, 1);
});

test('a video lasting as long as its job still clears at its end', async () => {
  const { element, calls, clock } = await play({ jobId: 'job-1', durationSec: 12, media: video(12) });
  clock.remainingMs = 800;
  element.dispatch('ended');
  assert.deepEqual(calls, ['clearOverlay']);
  assert.equal(element.playCalls, 1);
});
