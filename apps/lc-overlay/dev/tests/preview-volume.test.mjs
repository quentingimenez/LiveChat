// The board's previews must sound exactly like the overlay: both turn the
// general and guild volume settings into the same media element volume. The
// overlay also has to re-apply that volume to the media already playing when a
// setting changes (general volume, guild volume, media mute).

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyBootstrapUtils } from '../../web/dist/renderer/overlay/legacy-bootstrap-utils.js';
import { createOverlayLegacyMediaDomUtils } from '../../web/dist/renderer/overlay/legacy-media-dom-utils.js';
import { createOverlayLegacyRuntimeRenderHandlersUtils } from '../../web/dist/renderer/overlay/legacy-runtime-render-handlers-utils.js';
import { createOverlayLegacyUtils, VOLUME_CURVE_GAMMA } from '../../web/dist/renderer/overlay/legacy-utils.js';

// Minimal DOM stand-ins for the renderer's `instanceof` checks.
class FakeElement {
  constructor() {
    this.style = { setProperty() {} };
  }
}
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
const bootstrapUtils = createOverlayLegacyBootstrapUtils();
const mediaDomUtils = createOverlayLegacyMediaDomUtils();
const { createRuntimeRenderHandlers } = createOverlayLegacyRuntimeRenderHandlersUtils();

// The overlay renderer around one media of guild `g1`, as the shell's
// `overlay://play` and `overlay://settings` describe it.
function createOverlayRenderer() {
  const element = new FakeMediaElement();
  const mediaLayer = new FakeElement();
  mediaLayer.querySelectorAll = () => [element];
  const state = {
    overlayConfig: { volume: 1, showText: true },
    activeGuildId: 'g1',
    activeVolume: null,
    activeShowText: null,
    activeLayout: null,
    mediaFrameState: {},
  };
  const handlers = createRuntimeRenderHandlers({
    nodes: { overlayRoot: new FakeElement(), mediaLayer, textLayer: null, countdownLayer: null },
    state,
    constants: { volumeCurveGamma: VOLUME_CURVE_GAMMA },
    utils: overlayUtils,
    mediaDomUtils,
    bootstrapUtils,
    textUtils: {},
    scheduleMediaHeaderLayout: () => {},
    ensureMediaFrame: () => null,
    setTimeout: () => 0,
    clearTimeout: () => {},
  });
  return { element, state, applySettings: handlers.onSettings };
}

// What the overlay sets on its media element: the shell's settings carry the
// general and per-guild volumes (`guildVolumes` in src/config.rs); the
// renderer applies their product through its perceptual curve.
function overlayElementVolume(general, guild) {
  const renderer = createOverlayRenderer();
  renderer.applySettings({ volume: general, guildVolumes: { g1: guild } });
  return renderer.element.volume;
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

test('a settings change re-applies the volume to the media already playing', () => {
  const renderer = createOverlayRenderer();
  // The play was rendered at 50 %: the shell sent that effective volume.
  renderer.state.activeVolume = 0.5;

  // General slider to 100 %: the live settings describe the playing guild.
  renderer.applySettings({ volume: 1, guildVolumes: { g1: 1 } });
  assert.equal(renderer.element.volume, 1);

  // Guild slider to 50 % on top of the general 100 %.
  renderer.applySettings({ volume: 1, guildVolumes: { g1: 0.5 } });
  assert.ok(Math.abs(renderer.element.volume - mediaGain(1, 0.5)) < 1e-12);

  // General slider to 30 % on top of the guild 50 %.
  renderer.applySettings({ volume: 0.3, guildVolumes: { g1: 0.5 } });
  assert.ok(Math.abs(renderer.element.volume - mediaGain(0.3, 0.5)) < 1e-12);

  // « Couper le son » applies to the current media too, and lifts again.
  renderer.applySettings({ volume: 0.3, guildVolumes: { g1: 0.5 }, mediaMuted: true });
  assert.equal(renderer.element.volume, 0);
  renderer.applySettings({ volume: 0.3, guildVolumes: { g1: 0.5 }, mediaMuted: false });
  assert.ok(Math.abs(renderer.element.volume - mediaGain(0.3, 0.5)) < 1e-12);
});

test('a play keeps its own volume when the settings do not describe its guild', () => {
  const renderer = createOverlayRenderer();
  // Older shell payload: no `guildVolumes`, the play's volume stays the source.
  renderer.state.activeVolume = 0.5;
  renderer.state.activeGuildId = 'gone';
  renderer.applySettings({ volume: 1 });
  assert.ok(Math.abs(renderer.element.volume - mediaGain(0.5, 1)) < 1e-12);
});
