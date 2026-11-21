// Settings descriptions: a hint that fits on one line keeps the width it
// needs; --measure (62ch) is only the fallback for a sentence that has to
// wrap. The overlay tests run without a DOM, so this pins the pure decision,
// the CSS fallback and the render/resize wiring.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const css = read('styles/settings.css');
const measure = read('core/measure.js');
const view = read('views/view.js');
const main = read('main.js');

test('the reading measure stays the fallback for a wrapped hint', () => {
  const hint = /\.settings-hint\s*\{([^}]*)\}/.exec(css);
  assert.ok(hint, 'the .settings-hint rule is missing');
  assert.match(hint[1], /max-width:\s*var\(--measure\)/);
});

test('a hint that fits drops the measure', () => {
  assert.match(css, /\.settings-hint\.is-one-line[^{}]*\{[^}]*max-width:\s*none/);
});

test('the one-line class is measured against the container', () => {
  assert.match(measure, /whiteSpace\s*=\s*'nowrap'/);
  assert.match(measure, /classList\.toggle\('is-one-line'/);
  assert.match(measure, /parent\.clientWidth/);
});

test('the selector covers hints, descriptions and admin form hints', () => {
  assert.match(measure, /\.settings-hint/);
  assert.match(measure, /\.settings-desc/);
  assert.match(measure, /\.form \.hint/);
});

test('every view render measures the descriptions', () => {
  assert.match(view, /fitSingleLineDescriptions\(view\)/);
  assert.match(view, /from '\.\.\/core\/measure\.js'/);
});

test('a window resize measures the current view again', () => {
  assert.match(main, /initSingleLineDescriptions\(\)/);
  assert.match(measure, /addEventListener\('resize'/);
  assert.match(measure, /requestAnimationFrame/);
  assert.match(measure, /getElementById\('view'\)/);
});

test('the decision keeps a line that fits, within a sub-pixel', async () => {
  const { fitsOnOneLine } = await import('../../web/app/core/measure.js');
  assert.equal(fitsOnOneLine(300, 400), true);
  assert.equal(fitsOnOneLine(400, 400), true);
  assert.equal(fitsOnOneLine(400.4, 400), true);
  assert.equal(fitsOnOneLine(400.6, 400), false);
  assert.equal(fitsOnOneLine(401, 400), false);
});
