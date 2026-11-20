// A display resolution change resizes the overlay window while it runs. A
// media sized in pixels by its display scale must then be re-fitted to the new
// zone, and the header chips re-laid out. Regression: the scale was applied
// once, at render time, and the window resize only re-fitted the header, so a
// scaled media kept the size measured for the old screen and drifted out of it.

import assert from 'node:assert/strict';
import test from 'node:test';

import { createOverlayLegacyBootstrapUtils } from '../../web/dist/renderer/overlay/legacy-bootstrap-utils.js';
import { createOverlayLegacyMediaRenderUtils } from '../../web/dist/renderer/overlay/legacy-media-render-utils.js';
import { createOverlayLegacyRuntimeRenderHandlersUtils } from '../../web/dist/renderer/overlay/legacy-runtime-render-handlers-utils.js';
import { createOverlayLegacyUtils, VOLUME_CURVE_GAMMA } from '../../web/dist/renderer/overlay/legacy-utils.js';

// Minimal DOM stand-ins for the renderer's `instanceof` checks.
class FakeStyle {
  constructor() {
    this.values = new Map();
  }
  set width(value) {
    this.values.set('width', value);
  }
  get width() {
    return this.values.get('width') ?? '';
  }
  set height(value) {
    this.values.set('height', value);
  }
  get height() {
    return this.values.get('height') ?? '';
  }
  setProperty(name, value) {
    this.values.set(name, String(value));
  }
  getPropertyValue(name) {
    return this.values.get(name) ?? '';
  }
}
class FakeElement {
  constructor() {
    this.style = new FakeStyle();
    this.isConnected = true;
    this.listeners = {};
  }
  addEventListener(name, handler) {
    (this.listeners[name] ||= []).push(handler);
  }
}
class FakeImage extends FakeElement {
  constructor(width, height) {
    super();
    this.naturalWidth = width;
    this.naturalHeight = height;
  }
}
class FakeLayer extends FakeElement {
  constructor(width, height) {
    super();
    this.clientWidth = width;
    this.clientHeight = height;
  }
}
globalThis.HTMLElement = FakeElement;
globalThis.HTMLImageElement = FakeImage;
globalThis.HTMLVideoElement = class extends FakeElement {};
globalThis.window = {
  requestAnimationFrame(callback) {
    callback();
    return 0;
  },
  getComputedStyle() {
    return { marginBottom: '0px' };
  },
};

const { renderMedia } = createOverlayLegacyMediaRenderUtils();

// Runs the real `renderMedia` on a 2000×1000 picture in a `layerWidth` ×
// `layerHeight` zone. The frame has no header/footer here: the zone is the
// whole space left for the picture.
async function renderPicture(layerWidth, layerHeight) {
  const mediaLayer = new FakeLayer(layerWidth, layerHeight);
  const frame = new FakeElement();
  const image = new FakeImage(2000, 1000);
  let headerLayouts = 0;
  const relayout = await renderMedia({
    payload: {
      jobId: 'job-1',
      media: { kind: 'image', url: 'https://media.test/pic', displayScalePercent: 100 },
    },
    buildAuthorizedMediaUrl: (url) => url,
    setActiveObjectUrl: () => {},
    createMediaElement: () => image,
    attachMediaDiagnostics: () => {},
    ensureMediaFrame: () => frame,
    mediaLayer,
    attachMediaElement: () => true,
    scheduleMediaHeaderLayout: () => {
      headerLayouts += 1;
    },
    applyVolume: () => {},
    setActivePlayableElement: () => {},
    applyMediaStartOffset: () => {},
    resolveStartOffsetSec: () => null,
    resumeCountdown: () => {},
    pauseCountdown: () => {},
    setPlaybackState: () => {},
    clearOverlay: () => {},
    videoExtensionSec: () => 0,
    resolveVideoEndAction: () => 'clear',
    videoHasAudioTrack: () => false,
    getRemainingCountdownMs: () => 1000,
    setCountdownAutoClear: () => {},
    logAutoplayError: () => {},
  });
  return { mediaLayer, image, relayout, headerLayouts: () => headerLayouts };
}

test('a scaled picture is re-fitted after the screen resolution changes (regression: it kept the old size)', async () => {
  const { mediaLayer, image, relayout, headerLayouts } = await renderPicture(800, 600);
  // 2000×1000 at 100 % is capped by the 800×600 zone.
  assert.equal(image.style.width, '800px');
  assert.equal(image.style.height, '400px');

  // The window shrank with the screen: the zone halves. Without a re-fit the
  // picture keeps 800×400 and overflows it.
  assert.equal(typeof relayout, 'function', 'renderMedia must hand back the re-fit');
  const before = headerLayouts();
  mediaLayer.clientWidth = 400;
  mediaLayer.clientHeight = 300;
  relayout();
  assert.equal(image.style.width, '400px');
  assert.equal(image.style.height, '200px');
  // The header chips are re-fitted to the new zone as well.
  assert.ok(headerLayouts() > before);
});

test('a picture without a display scale has nothing to re-fit', async () => {
  const media = { kind: 'image', url: 'https://media.test/pic' };
  const mediaLayer = new FakeLayer(800, 600);
  const image = new FakeImage(2000, 1000);
  const relayout = await renderMedia({
    payload: { jobId: 'job-1', media },
    buildAuthorizedMediaUrl: (url) => url,
    setActiveObjectUrl: () => {},
    createMediaElement: () => image,
    attachMediaDiagnostics: () => {},
    ensureMediaFrame: () => new FakeElement(),
    mediaLayer,
    attachMediaElement: () => true,
    scheduleMediaHeaderLayout: () => {},
    applyVolume: () => {},
    setActivePlayableElement: () => {},
    applyMediaStartOffset: () => {},
    resolveStartOffsetSec: () => null,
    resumeCountdown: () => {},
    pauseCountdown: () => {},
    setPlaybackState: () => {},
    clearOverlay: () => {},
    videoExtensionSec: () => 0,
    resolveVideoEndAction: () => 'clear',
    videoHasAudioTrack: () => false,
    getRemainingCountdownMs: () => 1000,
    setCountdownAutoClear: () => {},
    logAutoplayError: () => {},
  });
  assert.equal(relayout, null);
  // The CSS fit takes over, as before.
  assert.equal(image.style.width, '');
});

test('the render handlers replay the re-fit of the media being played', async () => {
  const relayoutCalls = [];
  const state = {
    overlayConfig: { showText: true },
    activeGuildId: null,
    activeVolume: null,
    activeShowText: null,
    activeLayout: null,
    mediaFrameState: {},
  };
  const { createRuntimeRenderHandlers } = createOverlayLegacyRuntimeRenderHandlersUtils();
  const handlers = createRuntimeRenderHandlers({
    nodes: {
      overlayRoot: new FakeElement(),
      mediaLayer: new FakeLayer(800, 600),
      textLayer: null,
      countdownLayer: null,
    },
    state,
    constants: { volumeCurveGamma: VOLUME_CURVE_GAMMA },
    utils: createOverlayLegacyUtils(),
    bootstrapUtils: createOverlayLegacyBootstrapUtils(),
    textUtils: {},
    mediaDomUtils: { applyMediaVolume: () => {} },
    mediaOffsetUtils: { applyMediaStartOffset: () => {} },
    diagnosticsUtils: { attachMediaDiagnostics: () => {} },
    playbackUtils: { videoExtensionSec: () => 0, resolveVideoEndAction: () => 'clear' },
    inlineVideoUtils: { videoHasAudioTrack: () => false },
    // The rendered media hands back its re-fit closure.
    mediaRenderUtils: {
      renderMedia: async () => () => {
        relayoutCalls.push(state.activeGuildId);
      },
    },
    playFlowUtils: {
      runPlayFlow: async (params) => {
        await params.renderMedia(params.payload);
      },
    },
    scheduleMediaHeaderLayout: () => {},
    ensureMediaFrame: () => null,
    setTimeout: () => 0,
    clearTimeout: () => {},
  });

  // No media rendered (or it had no scale): nothing to re-fit.
  assert.equal(handlers.relayoutActiveMedia(), false);

  await handlers.onPlay({ test: true, guildId: 'g1', media: { kind: 'image', url: 'x' } });
  assert.equal(handlers.relayoutActiveMedia(), true);
  assert.deepEqual(relayoutCalls, ['g1']);

  // A settings change moves the display zone too: the playing media is
  // re-fitted to it, like the header chips.
  handlers.onSettings({ showText: true });
  assert.deepEqual(relayoutCalls, ['g1', 'g1']);
});

test('a window resize replays the media re-fit and the header layout', async () => {
  let resizeHandler = null;
  const calls = [];
  await createOverlayLegacyBootstrapUtils().bootstrapOverlay({
    getConfig: async () => ({}),
    getCurrentConfig: () => ({}),
    overlayRoot: new FakeElement(),
    setConfig: () => {},
    registerOnPlay: () => {},
    registerOnStop: () => {},
    registerOnSettings: () => {},
    onPlay: () => {},
    onStop: () => {},
    onSettings: () => {},
    addWindowResizeListener: (handler) => {
      resizeHandler = handler;
    },
    scheduleMediaHeaderLayout: () => calls.push('header'),
    relayoutActiveMedia: () => calls.push('media'),
    ensureTwitterWidgets: async () => {},
    rendererReady: () => {},
  });

  assert.equal(typeof resizeHandler, 'function');
  resizeHandler();
  assert.deepEqual(calls, ['media', 'header']);
});
