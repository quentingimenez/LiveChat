// The zoomed stage scrolls virtually: no native scrollbar, so nothing that
// appears with the overflow can feed back into the zoomed size.
//
// When that room was read from a native scroller, the horizontal scrollbar
// appeared, the frame lost its height, the picture shrank, the scrollbar went,
// the picture grew back... and Chromium flipped between the two sizes on every
// frame: the picture looked doubled. And a video element that moves on screen
// loses Chromium's hardware overlay: on a wide-gamut screen it looks washed
// out for the whole gesture. In `windowed` mode the content is a fixed window
// and only the view it is told to show moves.
//
// No DOM under node:test: a minimal fake stands in for the few nodes the
// module builds, and the test plays the browser's part around it.

import assert from 'node:assert/strict';
import test from 'node:test';

class FakeNode {
  constructor(tag) {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.style = {};
    this.attributes = {};
    this.listeners = {};
    this.hidden = false;
    this.textContent = '';
    this.title = '';
    this.className = '';
    this.dataset = {};
    this.clientWidth = 0;
    this.clientHeight = 0;
    const classes = new Set();
    this.classList = {
      add: (name) => classes.add(name),
      remove: (name) => classes.delete(name),
      contains: (name) => classes.has(name),
    };
  }

  append(...nodes) {
    this.children.push(...nodes);
  }

  setAttribute(name, value) {
    this.attributes[name] = value;
    if (name === 'hidden') {
      this.hidden = true;
    }
  }

  addEventListener(type, listener) {
    (this.listeners[type] ||= []).push(listener);
  }

  removeEventListener() {}

  setPointerCapture() {}

  dispatch(type, event) {
    for (const listener of this.listeners[type] || []) {
      listener({ preventDefault() { this.defaultPrevented = true; }, stopPropagation() {}, target: this, ...event });
    }
  }

  getBoundingClientRect() {
    return { left: 0, top: 0, width: this.clientWidth, height: this.clientHeight };
  }
}

const observers = [];
let frames = [];

globalThis.Node = FakeNode;
globalThis.document = {
  createElement: (tag) => new FakeNode(tag),
  createTextNode: (text) => Object.assign(new FakeNode('#text'), { textContent: text }),
};
globalThis.ResizeObserver = class {
  constructor(callback) {
    this.callback = callback;
    observers.push(this);
  }

  observe() {}

  disconnect() {}
};
globalThis.requestAnimationFrame = (callback) => {
  frames.push(callback);
  return frames.length;
};
globalThis.cancelAnimationFrame = () => {};

const { STAGE_BAR, STAGE_PADDING, createStageZoom } = await import('../../web/app/views/editor/stage-zoom.js');

const WIDTH = 2184;
const HEIGHT = 1008;
const ROOM = { width: WIDTH - STAGE_BAR, height: HEIGHT - STAGE_BAR };

// The editor's stage frame, with a 16:9 picture sized from the room at
// `fit × zoom`, as both editors do.
function mountStage({ windowed = false, width = WIDTH, height = HEIGHT } = {}) {
  const content = new FakeNode('div');
  const views = [];
  const pans = [];
  let stage = null;
  stage = createStageZoom({
    content,
    windowed,
    onResize: () => {
      const room = stage.available();
      const scale = Math.min(room.width / 16, room.height / 9) * stage.zoom();
      views.push(stage.fit({ width: scale * 16, height: scale * 9 }));
    },
    onPan: (view) => pans.push(view),
  });
  Object.assign(stage.node, { clientWidth: width, clientHeight: height });
  observers[observers.length - 1].callback([]);
  return { stage, content, views, pans, viewport: stage.node };
}

const bars = (viewport) => viewport.children.filter((node) => node.className.includes('stage-zoom-bar'));

function middleDrag(viewport, dx, dy) {
  viewport.dispatch('pointerdown', { button: 1, buttons: 4, clientX: 500, clientY: 500, pointerId: 1 });
  viewport.dispatch('pointermove', { button: -1, buttons: 4, clientX: 500 + dx, clientY: 500 + dy, pointerId: 1 });
  viewport.dispatch('pointerup', { button: 1, buttons: 0, clientX: 500 + dx, clientY: 500 + dy, pointerId: 1 });
}

function runFrames(count = 60) {
  let now = 0;
  for (let index = 0; index < count && frames.length; index += 1) {
    const pending = frames;
    frames = [];
    now += 16;
    for (const callback of pending) {
      callback(now);
    }
  }
}

test('the room for the content does not depend on the zoom or the overflow', () => {
  const { stage } = mountStage();
  const fit = stage.available();
  assert.deepEqual(fit, { width: ROOM.width - STAGE_PADDING * 2, height: ROOM.height - STAGE_PADDING * 2 });
  stage.setZoom(1.25);
  assert.deepEqual(stage.available(), fit);
  stage.setZoom(4);
  assert.deepEqual(stage.available(), fit);
});

test('the zoomed size settles: one layout per zoom step, never two sizes', () => {
  const { stage, views } = mountStage();
  views.length = 0;
  stage.setZoom(1.25);
  observers[observers.length - 1].callback([]);
  assert.equal(new Set(views.map((view) => `${view.width}x${view.height}`)).size, 1);
});

test('a frame smaller than its margins never gives a negative room', () => {
  const { stage } = mountStage({ width: 30, height: 30 });
  assert.deepEqual(stage.available(), { width: 0, height: 0 });
});

test('at x1 the picture is centred, whole and without scrollbars', () => {
  const { stage, content, viewport, views } = mountStage();
  const view = views.at(-1);
  assert.equal(view.x, 0);
  assert.equal(view.y, 0);
  assert.ok(view.width <= ROOM.width - STAGE_PADDING * 2 + 0.001);
  assert.ok(view.height <= ROOM.height - STAGE_PADDING * 2 + 0.001);
  assert.equal(
    content.style.transform,
    `translate(${Math.round((ROOM.width - view.width) / 2)}px, ${Math.round((ROOM.height - view.height) / 2)}px)`,
  );
  assert.ok(bars(viewport).every((bar) => bar.hidden));
  assert.equal(stage.zoom(), 1);
});

test('zoomed, the view is the room and the scrollbars show', () => {
  const { stage, viewport, views } = mountStage();
  stage.setZoom(2);
  const view = views.at(-1);
  assert.equal(view.width, ROOM.width);
  assert.equal(view.height, ROOM.height);
  assert.ok(bars(viewport).every((bar) => !bar.hidden));
});

test('windowed: a pan moves the view, never the content', () => {
  const { stage, content, viewport, pans } = mountStage({ windowed: true });
  stage.setZoom(2);
  const placed = content.style.transform;
  assert.equal(placed, 'translate(0px, 0px)');
  middleDrag(viewport, -120, -80);
  assert.equal(content.style.transform, placed);
  const view = pans.at(-1);
  assert.ok(view, 'the editor must be told about the pan');
  assert.equal(view.width, ROOM.width);
  assert.equal(view.height, ROOM.height);
  // The content follows the hand: dragging left and up shows what is right
  // and below.
  assert.ok(view.x > 0);
  assert.ok(view.y > 0);
});

test('not windowed: the content slides under the frame by the scroll', () => {
  const { stage, content, viewport, pans } = mountStage();
  stage.setZoom(2);
  const before = pans.length;
  middleDrag(viewport, -120, -80);
  const view = pans.at(-1);
  assert.ok(pans.length > before);
  assert.equal(content.style.transform, `translate(${-Math.round(view.x)}px, ${-Math.round(view.y)}px)`);
});

test('the scroll stops at the edges of the zoomed content', () => {
  const { stage, viewport, pans, views } = mountStage({ windowed: true });
  stage.setZoom(2);
  const full = views.at(-1);
  middleDrag(viewport, -300, -300);
  middleDrag(viewport, 50000, 50000);
  assert.deepEqual([pans.at(-1).x, pans.at(-1).y], [0, 0]);
  middleDrag(viewport, -50000, -50000);
  const fullWidth = (Math.min((ROOM.width - 48) / 16, (ROOM.height - 48) / 9) * 2) * 16;
  const fullHeight = (Math.min((ROOM.width - 48) / 16, (ROOM.height - 48) / 9) * 2) * 9;
  assert.ok(Math.abs(pans.at(-1).x - (fullWidth - ROOM.width)) < 1e-6);
  assert.ok(Math.abs(pans.at(-1).y - (fullHeight - ROOM.height)) < 1e-6);
  assert.equal(full.width, ROOM.width);
});

test('Ctrl + wheel zooms around the pointer', () => {
  const { stage, viewport, views } = mountStage({ windowed: true });
  stage.setZoom(2);
  const before = views.at(-1);
  // The content point under the pointer, before and after one step.
  const pointer = { x: 700, y: 300 };
  const fullBefore = stage.extent();
  const onContent = { x: (before.x + pointer.x) / fullBefore.width, y: (before.y + pointer.y) / fullBefore.height };
  viewport.dispatch('wheel', { ctrlKey: true, deltaY: -100, deltaMode: 0, clientX: pointer.x, clientY: pointer.y });
  assert.ok(stage.zoom() > 2);
  const after = views.at(-1);
  const fullAfter = stage.extent();
  assert.ok(Math.abs((after.x + pointer.x) / fullAfter.width - onContent.x) < 1e-9);
  assert.ok(Math.abs((after.y + pointer.y) / fullAfter.height - onContent.y) < 1e-9);
});

test('the wheel scrolls a zoomed stage and leaves a fitted one alone', () => {
  const fitted = mountStage({ windowed: true });
  const event = { deltaY: 100, deltaX: 0, deltaMode: 0 };
  let prevented = false;
  fitted.viewport.dispatch('wheel', { ...event, preventDefault: () => { prevented = true; } });
  runFrames();
  assert.equal(prevented, false);
  assert.equal(fitted.pans.length, 0);

  const zoomed = mountStage({ windowed: true });
  zoomed.stage.setZoom(2);
  zoomed.viewport.dispatch('wheel', { ...event, preventDefault: () => { prevented = true; } });
  runFrames();
  assert.equal(prevented, true);
  assert.ok(Math.abs(zoomed.pans.at(-1).y - 100) < 0.5, `scrolled to ${zoomed.pans.at(-1).y}`);
  assert.equal(zoomed.pans.at(-1).x, 0);
});

test('reset brings the fitted picture back, scroll and all', () => {
  const { stage, viewport, views } = mountStage({ windowed: true });
  stage.setZoom(3);
  middleDrag(viewport, -300, -300);
  stage.reset();
  assert.equal(stage.zoom(), 1);
  assert.equal(views.at(-1).x, 0);
  assert.equal(views.at(-1).y, 0);
  assert.ok(bars(viewport).every((bar) => bar.hidden));
});

test('a pan that cannot move does not lay the editor out again', () => {
  const { stage, viewport, pans } = mountStage({ windowed: true });
  stage.setZoom(2);
  // Already at the top left: dragging right and down has nothing to show.
  middleDrag(viewport, 200, 200);
  assert.equal(pans.length, 0);
});

test('a zoom step the editor could not lay out leaves no anchor behind', () => {
  // The editor skips `fit` while its media has no size yet; the next layout,
  // from a resize, must not jump to that old pointer.
  const content = new FakeNode('div');
  const views = [];
  let ready = true;
  let stage = null;
  stage = createStageZoom({
    content,
    windowed: true,
    onResize: () => {
      if (!ready) {
        return;
      }
      const room = stage.available();
      const scale = Math.min(room.width / 16, room.height / 9) * stage.zoom();
      views.push(stage.fit({ width: scale * 16, height: scale * 9 }));
    },
  });
  Object.assign(stage.node, { clientWidth: WIDTH, clientHeight: HEIGHT });
  const observer = observers[observers.length - 1];
  observer.callback([]);
  ready = false;
  stage.node.dispatch('wheel', { ctrlKey: true, deltaY: -300, deltaMode: 0, clientX: 2000, clientY: 900 });
  ready = true;
  observer.callback([]);
  assert.ok(stage.zoom() > 1);
  assert.deepEqual([views.at(-1).x, views.at(-1).y], [0, 0]);
});
