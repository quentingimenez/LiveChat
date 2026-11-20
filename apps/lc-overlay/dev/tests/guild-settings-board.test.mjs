// Layout of the « Board » section of the guild settings page. The quota input
// is a `.field` (width: 100%): nesting it in a shrink-to-fit row inside the
// control column squeezed the row to the input and wrapped the save button onto
// a second line. The input and its button therefore share the control column,
// and the destructive action sits in its own band, below a divider.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Declarations of the rule whose selector is exactly `selector`.
function ruleOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    if (match[1].replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) {
      return match[2];
    }
  }
  return null;
}

const board = () => {
  const source = read('views/guild-settings.js');
  const start = source.indexOf('function renderBoardSettings');
  assert.ok(start >= 0, 'renderBoardSettings not found');
  return source.slice(start);
};

test('the quota input and its save button share the control column', () => {
  // Regression: a `row is-wrap` around the input shrank to the input width and
  // pushed « Enregistrer » below it. The button belongs next to the field.
  assert.doesNotMatch(board(), /class: 'row is-wrap'/);
  assert.match(board(), /controlRow\(\s*'Quota propre \(Go\)',\s*\[quota, h\('button'/);
});

test('the destructive action is set apart from the form', () => {
  assert.match(board(), /class: 'settings-danger'/);
  assert.match(board(), /class: 'btn is-danger'/);
  assert.match(ruleOf(read('styles/settings.css'), '.settings-danger'), /border-top:\s*1px solid var\(--line\)/);
});

test('the occupation line introduces the section above the explanatory hint', () => {
  // `.settings-hint.is-summary` carries the current usage, `.is-lead` the
  // explanation below it: the two must not run together.
  const css = read('styles/settings.css');
  assert.match(board(), /settings-hint is-summary' \}, `Occupation/);
  assert.match(board(), /settings-hint is-lead' \},/);
  const summary = ruleOf(css, '.settings-hint.is-summary');
  assert.match(summary, /color:\s*var\(--text-soft\)/);
  assert.match(summary, /margin:\s*0 0 4px/);
});
