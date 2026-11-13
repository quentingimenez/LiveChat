// Unit tests of the zone placement geometry and of the overlay layout resolver.

import assert from 'node:assert/strict';
import test from 'node:test';

import { resolveLayout } from '../../web/dist/renderer/overlay/legacy-bootstrap-utils.js';
import {
  MIN_ZONE_FRACTION,
  moveRect,
  normalizeZone,
  nudgeRect,
  rectToZone,
  resizeRect,
  zoneToRect,
} from '../../web/dist/renderer/overlay/placement-geometry.js';

const SCREEN = { width: 2000, height: 1000 };

test('a zone and its rectangle convert both ways', () => {
  const zone = { x: 0.25, y: 0.1, width: 0.5, height: 0.4 };
  const rect = zoneToRect(zone, SCREEN);
  assert.deepEqual(rect, { left: 500, top: 100, width: 1000, height: 400 });
  assert.deepEqual(rectToZone(rect, SCREEN), zone);
});

test('a zone is normalized like the shell does', () => {
  assert.deepEqual(normalizeZone({ x: 0.9, y: 0.1, width: 0.3, height: 0.2 }), {
    x: 0.7,
    y: 0.1,
    width: 0.3,
    height: 0.2,
  });
  assert.deepEqual(normalizeZone({ x: 0, y: 0, width: 0.01, height: 2 }), {
    x: 0,
    y: 0,
    width: MIN_ZONE_FRACTION,
    height: 1,
  });
  assert.deepEqual(normalizeZone({ x: Number.NaN, y: 0, width: 1, height: 1 }), {
    x: 0,
    y: 0,
    width: 1,
    height: 1,
  });
});

test('moving keeps the zone on screen', () => {
  const start = { left: 100, top: 100, width: 400, height: 300 };
  assert.deepEqual(moveRect(start, -500, 2000, SCREEN).rect, { left: 0, top: 700, width: 400, height: 300 });
});

test('moving snaps an edge or the centre to the screen guides', () => {
  const start = { left: 100, top: 100, width: 400, height: 300 };
  // Right edge 5 px from the screen centre (1000).
  const toCentre = moveRect(start, 495, 0, SCREEN, { snap: true });
  assert.equal(toCentre.rect.left, 600);
  assert.deepEqual(toCentre.guides, [{ axis: 'x', at: 1000 }]);
  // Zone centre 6 px below the screen middle (500).
  const middle = moveRect(start, 0, 256, SCREEN, { snap: true });
  assert.equal(middle.rect.top, 350);
  assert.deepEqual(middle.guides, [{ axis: 'y', at: 500 }]);
  // Too far from any guide: no magnetism.
  const free = moveRect(start, 300, 0, SCREEN, { snap: true });
  assert.equal(free.rect.left, 400);
  assert.deepEqual(free.guides, []);
});

test('without magnetism the zone follows the pointer exactly', () => {
  const start = { left: 100, top: 100, width: 400, height: 300 };
  const result = moveRect(start, 495, 0, SCREEN, { snap: false });
  assert.equal(result.rect.left, 595);
  assert.deepEqual(result.guides, []);
});

test('a corner resizes from the opposite corner', () => {
  const start = { left: 500, top: 200, width: 400, height: 300 };
  assert.deepEqual(resizeRect(start, 'se', 100, 50, SCREEN).rect, {
    left: 500,
    top: 200,
    width: 500,
    height: 350,
  });
  assert.deepEqual(resizeRect(start, 'nw', -100, -50, SCREEN).rect, {
    left: 400,
    top: 150,
    width: 500,
    height: 350,
  });
});

test('an edge only resizes its own side', () => {
  const start = { left: 500, top: 200, width: 400, height: 300 };
  assert.deepEqual(resizeRect(start, 'e', 120, 999, SCREEN).rect, {
    left: 500,
    top: 200,
    width: 520,
    height: 300,
  });
  assert.deepEqual(resizeRect(start, 'n', 999, -150, SCREEN).rect, {
    left: 500,
    top: 50,
    width: 400,
    height: 450,
  });
});

test('resizing keeps the minimum size and the screen', () => {
  const start = { left: 500, top: 200, width: 400, height: 300 };
  const shrunk = resizeRect(start, 'se', -1000, -1000, SCREEN).rect;
  assert.equal(shrunk.width, MIN_ZONE_FRACTION * SCREEN.width);
  assert.equal(shrunk.height, MIN_ZONE_FRACTION * SCREEN.height);
  const grown = resizeRect(start, 'nw', -5000, -5000, SCREEN).rect;
  assert.deepEqual(grown, { left: 0, top: 0, width: 900, height: 500 });
});

test('a dragged edge snaps to the screen edge', () => {
  const start = { left: 500, top: 200, width: 400, height: 300 };
  const result = resizeRect(start, 'e', 1094, 0, SCREEN, { snap: true });
  assert.equal(result.rect.width, 1500);
  assert.deepEqual(result.guides, [{ axis: 'x', at: 2000 }]);
});

test('arrow keys move or resize by the given step', () => {
  const rect = { left: 10, top: 10, width: 400, height: 300 };
  assert.deepEqual(nudgeRect(rect, -1, 0, SCREEN), { left: 9, top: 10, width: 400, height: 300 });
  assert.deepEqual(nudgeRect(rect, 0, 10, SCREEN, { resize: true }), {
    left: 10,
    top: 10,
    width: 400,
    height: 310,
  });
  assert.deepEqual(nudgeRect({ left: 0, top: 0, width: 400, height: 300 }, -1, -1, SCREEN), {
    left: 0,
    top: 0,
    width: 400,
    height: 300,
  });
});

test('the overlay layout resolves the zone and the 9 alignments', () => {
  assert.deepEqual(resolveLayout({ zone: { x: 0.5, y: 0, width: 0.5, height: 1 }, align: 'bottom-right' }), {
    zone: { x: 0.5, y: 0, width: 0.5, height: 1 },
    align: 'bottom-right',
    alignItems: 'flex-end',
    justifyContent: 'flex-end',
  });
  assert.equal(resolveLayout({ align: 'left' }).justifyContent, 'flex-start');
  assert.equal(resolveLayout({ align: 'left' }).alignItems, 'center');
  const fallback = resolveLayout({ zone: { x: 0, y: 0, width: 0, height: 1 }, align: 'nowhere' });
  assert.deepEqual(fallback.zone, { x: 0, y: 0, width: 1, height: 1 });
  assert.equal(fallback.align, 'center');
  assert.deepEqual(resolveLayout(undefined).zone, { x: 0, y: 0, width: 1, height: 1 });
});
