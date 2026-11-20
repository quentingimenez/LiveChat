// Unit tests of the frame pump that makes the overlay preview follow a video
// at its real frame rate: the preview canvas is painted for each frame the
// media presents instead of on `timeupdate` (about four times a second).

import assert from 'node:assert/strict';
import test from 'node:test';

import { createPreviewFramePump } from '../../web/app/views/editor/preview-frames.js';

// Minimal stand-in for a `<video>`: one pending frame callback, like the media
// element while it is playing.
function fakeMedia({ paused = false, cancel = true } = {}) {
  let nextHandle = 1;
  let pending = null;
  const cancelled = [];
  const media = {
    paused,
    requests: 0,
    requestVideoFrameCallback(callback) {
      media.requests += 1;
      const handle = nextHandle++;
      pending = { handle, callback };
      return handle;
    },
    get scheduled() {
      return pending !== null;
    },
    emitFrame() {
      const entry = pending;
      pending = null;
      if (entry) {
        entry.callback(0, {});
      }
    },
  };
  if (cancel) {
    media.cancelVideoFrameCallback = (handle) => {
      cancelled.push(handle);
      if (pending && pending.handle === handle) {
        pending = null;
      }
    };
  }
  return { media, cancelled };
}

function setup(options = {}) {
  const { media, cancelled } = fakeMedia(options);
  let active = options.active ?? true;
  let paints = 0;
  const pump = createPreviewFramePump({
    media,
    onFrame: () => {
      paints += 1;
    },
    isActive: () => active,
  });
  return {
    media,
    cancelled,
    pump,
    paints: () => paints,
    setActive: (next) => {
      active = next;
    },
  };
}

test('a playing video is pumped frame by frame', () => {
  const harness = setup();
  harness.pump.start();
  assert.equal(harness.media.requests, 1);
  assert.equal(harness.media.scheduled, true);

  harness.media.emitFrame();
  assert.equal(harness.paints(), 1);
  // The pump asked for the next frame straight away.
  assert.equal(harness.media.requests, 2);
  assert.equal(harness.media.scheduled, true);
});

test('start is idempotent while the pump is running', () => {
  const harness = setup();
  harness.pump.start();
  harness.pump.start();
  assert.equal(harness.media.requests, 1);
  harness.media.emitFrame();
  assert.equal(harness.media.requests, 2);
});

test('stop cancels the pending frame and stops the pump', () => {
  const harness = setup();
  harness.pump.start();
  harness.pump.stop();
  assert.equal(harness.media.scheduled, false);
  assert.equal(harness.cancelled.length, 1);

  harness.media.emitFrame();
  assert.equal(harness.paints(), 0);
  assert.equal(harness.media.requests, 1);

  // Starting again schedules exactly one more frame.
  harness.pump.start();
  assert.equal(harness.media.requests, 2);
});

test('a paused media is never pumped', () => {
  const paused = setup({ paused: true });
  paused.pump.start();
  assert.equal(paused.media.requests, 0);
  assert.equal(paused.media.scheduled, false);

  // Pausing in the middle of a playback ends the pump on the next frame.
  const playing = setup();
  playing.pump.start();
  playing.media.paused = true;
  playing.media.emitFrame();
  assert.equal(playing.paints(), 0);
  assert.equal(playing.media.scheduled, false);

  playing.media.paused = false;
  playing.pump.start();
  assert.equal(playing.media.requests, 2);
});

test('an inactive preview is not painted', () => {
  const harness = setup({ active: false });
  harness.pump.start();
  assert.equal(harness.media.requests, 0);

  harness.setActive(true);
  harness.pump.start();
  harness.media.emitFrame();
  assert.equal(harness.paints(), 1);

  harness.setActive(false);
  harness.media.emitFrame();
  assert.equal(harness.paints(), 1);
  assert.equal(harness.media.scheduled, false);
});

test('a browser without requestVideoFrameCallback is left to the fallback', () => {
  const { media } = fakeMedia();
  delete media.requestVideoFrameCallback;
  let paints = 0;
  const pump = createPreviewFramePump({
    media,
    onFrame: () => {
      paints += 1;
    },
    isActive: () => true,
  });
  assert.equal(pump.supported, false);
  pump.start();
  assert.equal(media.requests, 0);
  assert.equal(paints, 0);
});

test('stop works when the browser cannot cancel a frame callback', () => {
  const harness = setup({ cancel: false });
  harness.pump.start();
  assert.doesNotThrow(() => harness.pump.stop());
  harness.media.emitFrame();
  assert.equal(harness.paints(), 0);
});
