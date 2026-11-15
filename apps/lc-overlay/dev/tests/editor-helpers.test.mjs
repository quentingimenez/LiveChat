import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  NEUTRAL_ADJUST,
  avEditProblem,
  buildRecipe,
  centeredCrop,
  cssFilter,
  cutRange,
  defaultAvEdit,
  dragCropRect,
  editedFileName,
  editedTitle,
  exportMimeFor,
  fadeGain,
  fitScale,
  formatClock,
  isWhole,
  keepRange,
  keptDuration,
  nextKeptTime,
  normalizeSegments,
  outputDuration,
  parseClock,
  recipeIsNoop,
  removedRanges,
  rotatedSize,
  sourceToOutputTime,
  wholeSegments,
} from '../../web/app/core/editor-helpers.js';

test('normalizeSegments: sorts, clamps, merges and drops empty ranges', () => {
  assert.deepEqual(normalizeSegments([[8, 12], [-1, 2], [2, 3], [5, 5]], 10), [
    [0, 3],
    [8, 10],
  ]);
  assert.deepEqual(normalizeSegments([], 10), []);
  assert.deepEqual(normalizeSegments([[1, 2]], 0), []);
});

test('keepRange: trims to the selection, whatever the order of its ends', () => {
  assert.deepEqual(keepRange(wholeSegments(10), 2, 6), [[2, 6]]);
  assert.deepEqual(keepRange(wholeSegments(10), 6, 2), [[2, 6]]);
  assert.deepEqual(keepRange([[0, 3], [5, 10]], 2, 6), [
    [2, 3],
    [5, 6],
  ]);
});

test('cutRange: removes the middle, an edge or a whole range', () => {
  assert.deepEqual(cutRange(wholeSegments(10), 4, 6), [
    [0, 4],
    [6, 10],
  ]);
  assert.deepEqual(cutRange(wholeSegments(10), 0, 3), [[3, 10]]);
  assert.deepEqual(cutRange([[0, 2], [4, 6], [8, 10]], 3, 7), [
    [0, 2],
    [8, 10],
  ]);
  assert.deepEqual(cutRange(wholeSegments(10), 0, 10), []);
});

test('keptDuration, isWhole and removedRanges describe the result', () => {
  const segments = [[1, 4], [6, 9]];
  assert.equal(keptDuration(segments), 6);
  assert.equal(isWhole(segments, 10), false);
  assert.equal(isWhole(wholeSegments(10), 10), true);
  assert.deepEqual(removedRanges(segments, 10), [
    [0, 1],
    [4, 6],
    [9, 10],
  ]);
  assert.deepEqual(removedRanges(wholeSegments(10), 10), []);
});

test('nextKeptTime: playback skips what was removed and stops after the end', () => {
  const segments = [[1, 4], [6, 9]];
  assert.equal(nextKeptTime(segments, 0), 1);
  assert.equal(nextKeptTime(segments, 2), 2);
  assert.equal(nextKeptTime(segments, 4), 6);
  assert.equal(nextKeptTime(segments, 5), 6);
  assert.equal(nextKeptTime(segments, 9), null);
});

test('sourceToOutputTime and outputDuration follow cuts and speed', () => {
  const segments = [[1, 4], [6, 9]];
  assert.equal(sourceToOutputTime(segments, 1, 0), 0);
  assert.equal(sourceToOutputTime(segments, 1, 3), 2);
  assert.equal(sourceToOutputTime(segments, 1, 5), 3);
  assert.equal(sourceToOutputTime(segments, 2, 7), 2);
  assert.equal(outputDuration(segments, 2), 3);
  assert.equal(outputDuration(segments, 0.5), 12);
});

test('fadeGain: ramps at both ends only', () => {
  assert.equal(fadeGain(0, 10, 2, 2), 0);
  assert.equal(fadeGain(1, 10, 2, 2), 0.5);
  assert.equal(fadeGain(5, 10, 2, 2), 1);
  assert.equal(fadeGain(9, 10, 2, 2), 0.5);
  assert.equal(fadeGain(5, 10, 0, 0), 1);
});

test('buildRecipe: an untouched media gives an empty recipe', () => {
  const recipe = buildRecipe(defaultAvEdit(12.5), 12.5);
  assert.deepEqual(recipe, {});
  assert.equal(recipeIsNoop(recipe), true);
});

test('buildRecipe: only what changed is sent', () => {
  const edit = {
    ...defaultAvEdit(10),
    segments: [[1.23456, 4], [6, 9]],
    speed: 1.5,
    volume: 2,
    fadeIn: 0.5,
    reverse: true,
    crop: { x: 0.1, y: 0, width: 0.8, height: 1 },
    rotate: 450,
    flipH: true,
    output: 'gif',
  };
  assert.deepEqual(buildRecipe(edit, 10), {
    segments: [
      [1.235, 4],
      [6, 9],
    ],
    speed: 1.5,
    volume: 2,
    fadeInSec: 0.5,
    reverse: true,
    crop: { x: 0.1, y: 0, width: 0.8, height: 1 },
    rotate: 90,
    flipH: true,
    output: 'gif',
  });
});

test('buildRecipe: a muted video sends neither volume nor fades', () => {
  const edit = { ...defaultAvEdit(10), mute: true, volume: 2, fadeIn: 1, fadeOut: 1 };
  assert.deepEqual(buildRecipe(edit, 10), { mute: true });
});

test('buildRecipe: a full-frame crop is no crop', () => {
  const edit = { ...defaultAvEdit(10), crop: { x: 0, y: 0, width: 1, height: 1 } };
  assert.deepEqual(buildRecipe(edit, 10), {});
});

test('avEditProblem: refuses an empty result and a long reverse', () => {
  assert.equal(avEditProblem(defaultAvEdit(10), 10), null);
  assert.match(avEditProblem({ ...defaultAvEdit(10), segments: [] }, 10), /presque rien/);
  assert.match(avEditProblem({ ...defaultAvEdit(300), reverse: true }, 300), /2 minutes/);
  assert.equal(avEditProblem({ ...defaultAvEdit(300), reverse: true, segments: [[0, 60]] }, 300), null);
});

test('formatClock and parseClock agree', () => {
  assert.equal(formatClock(0), '0:00,0');
  assert.equal(formatClock(7.26), '0:07,3');
  assert.equal(formatClock(59.96), '1:00,0');
  assert.equal(formatClock(3723.5), '1:02:03,5');
  assert.equal(parseClock('0:07,3'), 7.3);
  assert.equal(parseClock('83.4'), 83.4);
  assert.equal(parseClock('1:02:03'), 3723);
  assert.equal(parseClock(''), null);
  assert.equal(parseClock('abc'), null);
  assert.equal(parseClock('1::2'), null);
});

test('centeredCrop: largest centered rectangle of a ratio', () => {
  assert.deepEqual(centeredCrop(1, 200, 100), { x: 50, y: 0, width: 100, height: 100 });
  assert.deepEqual(centeredCrop(2, 100, 100), { x: 0, y: 25, width: 100, height: 50 });
  assert.deepEqual(centeredCrop(0, 100, 80), { x: 0, y: 0, width: 100, height: 80 });
});

test('dragCropRect: moving stays inside the bounds', () => {
  const bounds = { width: 100, height: 100 };
  assert.deepEqual(dragCropRect({ x: 10, y: 10, width: 50, height: 50 }, 'move', 80, -30, bounds), {
    x: 50,
    y: 0,
    width: 50,
    height: 50,
  });
});

test('dragCropRect: a free resize moves only the dragged sides', () => {
  const bounds = { width: 100, height: 100 };
  assert.deepEqual(dragCropRect({ x: 10, y: 10, width: 50, height: 50 }, 'se', 20, 10, bounds), {
    x: 10,
    y: 10,
    width: 70,
    height: 60,
  });
  assert.deepEqual(dragCropRect({ x: 10, y: 10, width: 50, height: 50 }, 'nw', -50, 100, bounds, 0, 8), {
    x: 0,
    y: 52,
    width: 60,
    height: 8,
  });
});

test('dragCropRect: a locked ratio is kept and never leaves the bounds', () => {
  const bounds = { width: 100, height: 100 };
  const corner = dragCropRect({ x: 0, y: 0, width: 40, height: 40 }, 'se', 100, 10, bounds, 1);
  assert.deepEqual(corner, { x: 0, y: 0, width: 100, height: 100 });
  const side = dragCropRect({ x: 20, y: 20, width: 40, height: 20 }, 'e', 20, 0, bounds, 2);
  assert.deepEqual(side, { x: 20, y: 15, width: 60, height: 30 });
  const anchored = dragCropRect({ x: 50, y: 50, width: 40, height: 40 }, 'nw', -80, -80, bounds, 1);
  assert.deepEqual(anchored, { x: 0, y: 0, width: 90, height: 90 });
});

test('rotatedSize and fitScale', () => {
  assert.deepEqual(rotatedSize(300, 200, 1), { width: 200, height: 300 });
  assert.deepEqual(rotatedSize(300, 200, 2), { width: 300, height: 200 });
  assert.equal(fitScale(8000, 4000, 4000), 0.5);
  assert.equal(fitScale(800, 400, 4000), 1);
});

test('cssFilter: neutral adjustments draw the picture as it is', () => {
  assert.equal(cssFilter(NEUTRAL_ADJUST), 'none');
  assert.equal(cssFilter({ ...NEUTRAL_ADJUST, brightness: 120, preset: 'mono' }), 'brightness(1.2) grayscale(1)');
});

test('edited names: title suffix, export type and a safe file name', () => {
  assert.equal(editedTitle('Stonks'), 'Stonks (édité)');
  assert.equal(editedTitle('Stonks (édité)'), 'Stonks (édité)');
  assert.equal(editedTitle(''), 'Sans titre (édité)');
  assert.equal(exportMimeFor('image/jpeg'), 'image/jpeg');
  assert.equal(exportMimeFor('image/webp'), 'image/png');
  assert.equal(editedFileName('a/b\\c: d?', 'image/png'), 'a b c d.png');
  assert.equal(editedFileName('wait for it....', 'image/jpeg'), 'wait for it..jpg');
  assert.equal(editedFileName('', 'image/png'), 'image.png');
});
