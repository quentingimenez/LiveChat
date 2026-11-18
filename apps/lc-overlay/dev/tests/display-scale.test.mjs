// Unit tests of the image display scale: the pure size computation shared by
// the overlay renderer and the editor's live line.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  DEFAULT_DISPLAY_SCALE_PERCENT,
  displayScaleForStorage,
  displayedImageSize,
  normalizeDisplayScale,
  parseDisplayScale,
} from '../../web/app/core/editor-helpers.js';
import { computeDisplayedImageSize } from '../../web/dist/renderer/overlay/legacy-media-render-utils.js';

test('normalizeDisplayScale rounds and keeps whole percents in range', () => {
  assert.equal(normalizeDisplayScale(undefined), DEFAULT_DISPLAY_SCALE_PERCENT);
  assert.equal(normalizeDisplayScale(null), DEFAULT_DISPLAY_SCALE_PERCENT);
  assert.equal(normalizeDisplayScale('250'), 250);
  assert.equal(normalizeDisplayScale(250.6), 251);
  assert.equal(normalizeDisplayScale(0), 10);
  assert.equal(normalizeDisplayScale(5000), 1000);
});

test('a 100 % scale is stored as null', () => {
  assert.equal(displayScaleForStorage(100), null);
  assert.equal(displayScaleForStorage(10), 10);
  assert.equal(displayScaleForStorage(1000), 1000);
});

test('parseDisplayScale accepts whole percents from 10 to 1000', () => {
  for (const raw of ['10', '100', ' 250 ', 1000]) {
    assert.equal(parseDisplayScale(raw).ok, true, String(raw));
  }
  for (const raw of ['', '9', '1001', '2.5', 'abc', '-20']) {
    assert.deepEqual(parseDisplayScale(raw), { ok: false }, String(raw));
  }
});

test('displayedImageSize scales the natural picture without touching the ratio', () => {
  assert.deepEqual(
    displayedImageSize({ width: 400, height: 200, scalePercent: 250 }),
    { width: 1000, height: 500, capped: false },
  );
  assert.deepEqual(
    displayedImageSize({ width: 400, height: 200, scalePercent: 10 }),
    { width: 40, height: 20, capped: false },
  );
  // An unknown zone (`null`) never caps.
  assert.deepEqual(
    displayedImageSize({ width: 400, height: 200, scalePercent: 1000, maxWidth: null, maxHeight: null }),
    { width: 4000, height: 2000, capped: false },
  );
});

test('displayedImageSize caps to the zone and reports it', () => {
  assert.deepEqual(
    displayedImageSize({ width: 1920, height: 1080, scalePercent: 100, maxWidth: 1000, maxHeight: 1000 }),
    { width: 1000, height: 563, capped: true },
  );
  // The height limit is the one that applies.
  assert.deepEqual(
    displayedImageSize({ width: 1000, height: 2000, scalePercent: 200, maxWidth: 4000, maxHeight: 1500 }),
    { width: 750, height: 1500, capped: true },
  );
  assert.deepEqual(
    displayedImageSize({ width: 100, height: 50, scalePercent: 1000, maxWidth: 1920, maxHeight: 1080 }),
    { width: 1000, height: 500, capped: false },
  );
});

test('computeDisplayedImageSize behaves like the editor helper', () => {
  const cases = [
    { width: 400, height: 200, scalePercent: 250 },
    { width: 1920, height: 1080, scalePercent: 100, maxWidth: 1000, maxHeight: 1000 },
    { width: 1000, height: 2000, scalePercent: 200, maxWidth: 4000, maxHeight: 1500 },
    { width: 0, height: 0, scalePercent: 200 },
  ];
  for (const entry of cases) {
    const editor = displayedImageSize({
      width: entry.width,
      height: entry.height,
      scalePercent: entry.scalePercent,
      maxWidth: entry.maxWidth,
      maxHeight: entry.maxHeight,
    });
    const overlay = computeDisplayedImageSize({
      naturalWidth: entry.width,
      naturalHeight: entry.height,
      scalePercent: entry.scalePercent,
      maxWidth: entry.maxWidth,
      maxHeight: entry.maxHeight,
    });
    assert.deepEqual(overlay, editor);
  }
});
