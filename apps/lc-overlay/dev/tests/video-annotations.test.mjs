// Annotation recipe of the video editor: the drawing travels to the server as
// masks (fractions of the source frame) and as one PNG overlay, plus the
// colour adjustments. These tests pin the client half of the contract in
// `crates/lc-media/src/edit.rs`: fractions clamped to the frame, limits the
// server would reject applied here first, and a recipe that stays exactly the
// same as before when nothing was drawn.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MAX_MASK_STRENGTH,
  MAX_RECIPE_MASKS,
  NEUTRAL_ADJUST,
  adjustForRecipe,
  buildRecipe,
  masksForRecipe,
} from '../../web/app/core/editor-helpers.js';

const FRAME = { width: 320, height: 240 };

function mask(x1, y1, x2, y2, extra = {}) {
  return { type: 'mask', mode: 'pixel', strength: 16, x1, y1, x2, y2, ...extra };
}

test('masksForRecipe: a mask becomes a fraction of the source frame', () => {
  const masks = masksForRecipe([mask(32, 48, 192, 108)], FRAME.width, FRAME.height);
  assert.deepEqual(masks, [{ x: 0.1, y: 0.2, width: 0.5, height: 0.25, mode: 'pixel', strength: 0.05 }]);
});

test('masksForRecipe: strength is a length of the frame width, rounded to 4 decimals', () => {
  const masks = masksForRecipe([mask(0, 0, 32, 32, { strength: 3.333 })], FRAME.width, FRAME.height);
  assert.equal(masks[0].strength, 0.0104);
});

test('masksForRecipe: a rectangle drawn outside the frame is clamped to it', () => {
  const masks = masksForRecipe([mask(-40, -30, 80, 60)], FRAME.width, FRAME.height);
  assert.deepEqual(masks[0], { x: 0, y: 0, width: 0.25, height: 0.25, mode: 'pixel', strength: 0.05 });
});

test('masksForRecipe: a rectangle smaller than half a percent of a side is dropped', () => {
  assert.deepEqual(masksForRecipe([mask(100, 100, 101, 120)], FRAME.width, FRAME.height), []);
});

test('masksForRecipe: blur masks keep their mode, anything else is pixel', () => {
  const masks = masksForRecipe([mask(0, 0, 160, 120, { mode: 'blur' }), mask(0, 0, 160, 120, { mode: 'pixel' }), mask(0, 0, 160, 120, { mode: 'zzz' })], FRAME.width, FRAME.height);
  assert.deepEqual(masks.map((entry) => entry.mode), ['blur', 'pixel', 'pixel']);
});

test('masksForRecipe: strength above the server limit is clamped, not sent as is', () => {
  const masks = masksForRecipe([mask(0, 0, 160, 120, { strength: 400 })], FRAME.width, FRAME.height);
  assert.equal(masks[0].strength, MAX_MASK_STRENGTH);
});

test('masksForRecipe: at most the masks the server accepts', () => {
  const many = Array.from({ length: 25 }, () => mask(0, 0, 32, 24));
  assert.equal(masksForRecipe(many, FRAME.width, FRAME.height).length, MAX_RECIPE_MASKS);
});

test('masksForRecipe: other objects and an unknown frame size give nothing', () => {
  const objects = [mask(0, 0, 160, 120), { type: 'stroke', points: [0, 0, 10, 10] }, { type: 'text', text: 'salut' }];
  assert.equal(masksForRecipe(objects, FRAME.width, FRAME.height).length, 1);
  assert.deepEqual(masksForRecipe(objects, 0, 0), []);
  assert.deepEqual(masksForRecipe(objects, undefined, undefined), []);
  assert.deepEqual(masksForRecipe(undefined, FRAME.width, FRAME.height), []);
});

test('adjustForRecipe: neutral adjustments are left out of the recipe', () => {
  assert.equal(adjustForRecipe(undefined), null);
  assert.equal(adjustForRecipe({ ...NEUTRAL_ADJUST }), null);
});

test('adjustForRecipe: rounded percentages, one-decimal blur and the preset', () => {
  assert.deepEqual(adjustForRecipe({ brightness: 120, contrast: 90, saturate: 140, blur: 2.34, preset: 'sepia' }), {
    brightness: 120,
    contrast: 90,
    saturate: 140,
    blur: 2.3,
    preset: 'sepia',
  });
  assert.equal(adjustForRecipe({ brightness: 110, contrast: 100, saturate: 100, blur: 0 }).preset, 'none');
});

test('buildRecipe: without a drawing, the recipe is exactly the one of before', () => {
  const edit = {
    segments: [
      [0, 2],
      [4, 6],
    ],
    speed: 1.5,
    volume: 0.5,
    mute: false,
    fadeIn: 1,
    fadeOut: 0.5,
    reverse: true,
    crop: { x: 0.1, y: 0.2, width: 0.5, height: 0.25 },
    rotate: 90,
    flipH: true,
    flipV: false,
    output: 'gif',
  };
  const recipe = buildRecipe(edit, 10);
  assert.deepEqual(recipe, {
    segments: [
      [0, 2],
      [4, 6],
    ],
    speed: 1.5,
    volume: 0.5,
    fadeInSec: 1,
    fadeOutSec: 0.5,
    reverse: true,
    crop: { x: 0.1, y: 0.2, width: 0.5, height: 0.25 },
    rotate: 90,
    flipH: true,
    output: 'gif',
  });
  assert.equal(Object.keys(recipe).some((key) => key === 'masks' || key === 'overlay' || key === 'adjust'), false);
});

test('buildRecipe: a drawing adds its masks, its overlay and its adjustments', () => {
  const edit = {
    ...neutralEdit(),
    objects: [mask(0, 0, 160, 120), { type: 'text', text: 'salut', x: 160, y: 120, size: 30 }],
    adjust: { brightness: 120, contrast: 100, saturate: 100, blur: 0, preset: 'none' },
  };
  const recipe = buildRecipe(edit, 10, { width: FRAME.width, height: FRAME.height, overlay: 'iVBORw0KGgo=' });
  assert.deepEqual(recipe.masks, [{ x: 0, y: 0, width: 0.5, height: 0.5, mode: 'pixel', strength: 0.05 }]);
  assert.deepEqual(recipe.overlay, { png: 'iVBORw0KGgo=', width: 320, height: 240 });
  assert.deepEqual(recipe.adjust, { brightness: 120, contrast: 100, saturate: 100, blur: 0, preset: 'none' });
});

test('buildRecipe: an overlay needs a frame size, a drawing alone does not', () => {
  const edit = { ...neutralEdit(), objects: [mask(0, 0, 160, 120)] };
  const sized = buildRecipe(edit, 10, { width: FRAME.width, height: FRAME.height, overlay: 'iVBORw0KGgo=' });
  const unsized = buildRecipe(edit, 10, { overlay: 'iVBORw0KGgo=' });
  assert.equal(sized.overlay.width, 320);
  assert.equal('overlay' in unsized, false);
  assert.equal('masks' in unsized, false);
});

function neutralEdit() {
  return {
    segments: [[0, 10]],
    speed: 1,
    volume: 1,
    mute: false,
    fadeIn: 0,
    fadeOut: 0,
    reverse: false,
    crop: null,
    rotate: 0,
    flipH: false,
    flipV: false,
    output: 'same',
  };
}
