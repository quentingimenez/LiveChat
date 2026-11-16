// The board's previews must sound exactly like the overlay: both turn the
// general and guild volume settings into the same media element volume.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyMediaDomUtils } from '../../web/dist/renderer/overlay/legacy-media-dom-utils.js';
import { createOverlayLegacyRuntimeInputsUtils } from '../../web/dist/renderer/overlay/legacy-runtime-inputs-utils.js';
import { createOverlayLegacyUtils } from '../../web/dist/renderer/overlay/legacy-utils.js';

// Minimal DOM stand-ins for the renderer's `instanceof` checks.
class FakeElement {}
class FakeMediaElement extends FakeElement {
  constructor() {
    super();
    this.dataset = {};
    this.volume = 1;
    this.muted = false;
  }
}
globalThis.HTMLElement = FakeElement;
globalThis.HTMLMediaElement = FakeMediaElement;
globalThis.window = {};

const { mediaGain, previewVolume } = await import('../../web/app/core/sound.js');

const overlayUtils = createOverlayLegacyUtils();
const { applyMediaVolume } = createOverlayLegacyMediaDomUtils();
const { constants } = createOverlayLegacyRuntimeInputsUtils().createRuntimeInputs({ getElementById: () => null });

// What the overlay sets on its media element: the shell sends
// `clamp(general × guild)` in the `play` (`effective_volume` in
// src/net/ws.rs) and the renderer applies its perceptual curve.
function overlayElementVolume(general, guild) {
  const playVolume = Math.min(1, Math.max(0, general * guild));
  const element = new FakeMediaElement();
  const root = new FakeElement();
  root.querySelectorAll = () => [element];
  applyMediaVolume({
    root,
    perceptualGain: overlayUtils.toPerceptualGain(playVolume, constants.volumeCurveGamma),
  });
  return element.volume;
}

test('a preview plays at the level the overlay gives the same settings', () => {
  const steps = [0, 0.05, 0.1, 0.25, 0.3, 0.5, 0.75, 0.8, 1];
  for (const general of steps) {
    for (const guild of steps) {
      assert.equal(mediaGain(general, guild), overlayElementVolume(general, guild), `general ${general}, guild ${guild}`);
    }
  }
});

test('the preview follows the perceptual curve, not the linear product (regression: louder than the overlay)', () => {
  // 50 % used to give 0.5 in the preview against 0.218 on the overlay.
  assert.ok(Math.abs(mediaGain(0.5, 1) - 0.2176) < 0.001);
  assert.ok(Math.abs(mediaGain(1, 0.5) - 0.2176) < 0.001);
  assert.ok(mediaGain(0.8, 0.5) < 0.8 * 0.5);
  assert.equal(mediaGain(1, 1), 1);
  assert.equal(mediaGain(0, 1), 0);
});

test('missing or out-of-range settings are clamped like on the overlay', () => {
  assert.equal(mediaGain(1, undefined), 1);
  assert.equal(mediaGain(2, 3), 1);
  assert.equal(mediaGain(-1, 0.5), 0);
  assert.equal(mediaGain(0.5, undefined), overlayElementVolume(0.5, 1));
  // Default device settings: full volume for any guild.
  assert.equal(previewVolume('unknown-guild'), 1);
  assert.equal(previewVolume(null), 1);
});
