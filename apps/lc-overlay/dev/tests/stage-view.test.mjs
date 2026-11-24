// Geometry of the zoomed stage: the virtual scroll of the stage frame, the
// zoom anchored under the pointer, the wheel, and the part of the source
// frame a fixed video window shows.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  anchoredScroll,
  stageAxis,
  videoViewBox,
  wheelScrollDelta,
} from '../../web/app/core/editor-helpers.js';

test('stageAxis: content that fits is centred and never scrolls', () => {
  assert.deepEqual(stageAxis({ full: 600, room: 1000, scroll: 120 }), {
    overflow: false,
    scroll: 0,
    max: 0,
    start: 0,
    length: 600,
    offset: 200,
  });
});

test('stageAxis: overflowing content shows a room-wide window at the scroll', () => {
  assert.deepEqual(stageAxis({ full: 1600, room: 1000, scroll: 250 }), {
    overflow: true,
    scroll: 250,
    max: 600,
    start: 250,
    length: 1000,
    offset: -250,
  });
});

test('stageAxis: the scroll is clamped to the content', () => {
  assert.equal(stageAxis({ full: 1600, room: 1000, scroll: -40 }).scroll, 0);
  assert.equal(stageAxis({ full: 1600, room: 1000, scroll: 900 }).scroll, 600);
  assert.equal(stageAxis({ full: 1600, room: 1000, scroll: 900 }).start, 600);
});

test('stageAxis: half a pixel of overflow is not worth a scrollbar', () => {
  assert.equal(stageAxis({ full: 1000.4, room: 1000 }).overflow, false);
  assert.equal(stageAxis({ full: 1000.6, room: 1000 }).overflow, true);
});

test('anchoredScroll: the point under the pointer stays under it', () => {
  // 1600 px of content scrolled by 300: the pointer at 400 is on content 700,
  // that is 7/16 of it. At 3200 px that point is at 1400, so the scroll is 1000.
  const next = anchoredScroll({ pointer: 400, room: 1000, before: { full: 1600, scroll: 300 }, after: 3200 });
  assert.equal(next, 1000);
});

test('anchoredScroll: from a centred fit, the zoom grows around the pointer', () => {
  // 800 px centred in 1000: the content starts at 100. Pointer at 300 is on
  // a quarter of it; at 1600 px the quarter is at 400, so the scroll is 100.
  const next = anchoredScroll({ pointer: 300, room: 1000, before: { full: 800, scroll: 0 }, after: 1600 });
  assert.equal(next, 100);
});

test('anchoredScroll: the result stays inside the content', () => {
  assert.equal(anchoredScroll({ pointer: 990, room: 1000, before: { full: 1600, scroll: 600 }, after: 1200 }), 200);
  assert.equal(anchoredScroll({ pointer: 0, room: 1000, before: { full: 1600, scroll: 0 }, after: 900 }), 0);
});

test('wheelScrollDelta: pixels, lines and pages', () => {
  const room = { width: 800, height: 600 };
  assert.deepEqual(wheelScrollDelta({ deltaX: 3, deltaY: 100, deltaMode: 0 }, room), { x: 3, y: 100 });
  assert.deepEqual(wheelScrollDelta({ deltaX: 0, deltaY: 3, deltaMode: 1 }, room), { x: 0, y: 120 });
  assert.deepEqual(wheelScrollDelta({ deltaX: 1, deltaY: -1, deltaMode: 2 }, room), { x: 800, y: -600 });
});

test('wheelScrollDelta: Shift turns a vertical wheel into a horizontal scroll', () => {
  assert.deepEqual(wheelScrollDelta({ deltaX: 0, deltaY: 100, deltaMode: 0, shiftKey: true }, { width: 1, height: 1 }), { x: 100, y: 0 });
});

test('wheelScrollDelta: a touchpad already scrolling sideways keeps both axes with Shift', () => {
  assert.deepEqual(wheelScrollDelta({ deltaX: 30, deltaY: 5, deltaMode: 0, shiftKey: true }, { width: 1, height: 1 }), { x: 30, y: 5 });
});

const FRAME = { x: 0, y: 0, width: 1000, height: 500 };

test('videoViewBox: the whole picture shows the whole crop', () => {
  const crop = { x: 100, y: 50, width: 800, height: 400 };
  assert.deepEqual(videoViewBox({ crop, scale: 0.5, window: { x: 0, y: 0, width: 400, height: 200 } }), crop);
});

test('videoViewBox: a zoomed window shows its part of the crop', () => {
  const crop = { x: 100, y: 50, width: 800, height: 400 };
  // Displayed at 2 px per source pixel (1600 x 800): the window from (400, 200)
  // is the source rectangle from (100 + 200, 50 + 100), half its size.
  assert.deepEqual(
    videoViewBox({ crop, scale: 2, window: { x: 400, y: 200, width: 600, height: 300 } }),
    { x: 300, y: 150, width: 300, height: 150 },
  );
});

test('videoViewBox: a horizontal mirror shows the other side of the source', () => {
  assert.deepEqual(
    videoViewBox({ crop: FRAME, flipH: true, scale: 1, window: { x: 0, y: 0, width: 100, height: 500 } }),
    { x: 900, y: 0, width: 100, height: 500 },
  );
  assert.deepEqual(
    videoViewBox({ crop: FRAME, flipV: true, scale: 1, window: { x: 0, y: 0, width: 1000, height: 50 } }),
    { x: 0, y: 450, width: 1000, height: 50 },
  );
});

test('videoViewBox: a quarter turn brings the left of the source to the top', () => {
  // Turned clockwise, the 1000 x 500 frame shows as 500 x 1000 and its left
  // side is on top.
  assert.deepEqual(
    videoViewBox({ crop: FRAME, turns: 1, scale: 1, window: { x: 0, y: 0, width: 500, height: 100 } }),
    { x: 0, y: 0, width: 100, height: 500 },
  );
  // Three quarter turns put the right side on top.
  assert.deepEqual(
    videoViewBox({ crop: FRAME, turns: 3, scale: 1, window: { x: 0, y: 0, width: 500, height: 100 } }),
    { x: 900, y: 0, width: 100, height: 500 },
  );
  // A half turn shows the bottom right corner at the top left.
  assert.deepEqual(
    videoViewBox({ crop: FRAME, turns: 2, scale: 1, window: { x: 0, y: 0, width: 100, height: 50 } }),
    { x: 900, y: 450, width: 100, height: 50 },
  );
});

test('videoViewBox: the mirror applies to the turned picture', () => {
  // Turned, then mirrored: the left of the screen is the right of the turned
  // picture, which is the top of the source.
  assert.deepEqual(
    videoViewBox({ crop: FRAME, turns: 1, flipH: true, scale: 1, window: { x: 0, y: 0, width: 50, height: 1000 } }),
    { x: 0, y: 0, width: 1000, height: 50 },
  );
});
