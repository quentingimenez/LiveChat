// Shortcut settings rows: every row reserves the same key column before its
// « Modifier » and « Effacer » buttons, whether a shortcut is set or not.
// These invariants keep the two buttons on one vertical line and leave the
// key slot free to wrap a long accelerator without moving the actions.

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

const css = read('styles/settings.css');
const view = read('views/settings.js');

test('shortcut rows use a fixed key column and fixed action columns', () => {
  const control = ruleOf(css, '.settings-shortcut-row .settings-control') || '';
  assert.match(control, /display:\s*grid/);
  assert.match(control, /grid-template-columns:\s*128px max-content max-content/);
  assert.match(control, /column-gap:\s*10px/);
});

test('key, edit and clear cells have explicit grid columns', () => {
  const value = ruleOf(css, '.settings-shortcut-row .settings-shortcut-value') || '';
  assert.match(value, /grid-column:\s*1/);

  const edit = ruleOf(css, '.settings-shortcut-row .settings-shortcut-edit') || '';
  assert.match(edit, /grid-column:\s*2/);

  const clear = ruleOf(css, '.settings-shortcut-row .settings-shortcut-clear') || '';
  assert.match(clear, /grid-column:\s*3/);
});

test('a long shortcut wraps inside its fixed key column', () => {
  const value = ruleOf(css, '.settings-shortcut-row .settings-shortcut-value') || '';
  assert.match(value, /min-width:\s*0/);

  const key = ruleOf(css, '.settings-shortcut-row .settings-shortcut-value kbd') || '';
  assert.match(key, /white-space:\s*normal/);
  assert.match(key, /max-width:\s*100%/);
});

test('the row markup names the key and the two actions', () => {
  assert.match(view, /settings-row settings-shortcut-row/);
  assert.match(view, /class: 'settings-shortcut-value'/);
  assert.match(view, /class: 'btn settings-shortcut-edit'/);
  assert.match(view, /class: 'btn is-quiet settings-shortcut-clear'/);
});

test('capture hint and error span the whole control row', () => {
  const capture = ruleOf(css, '.settings-shortcut-row .settings-shortcut-capture') || '';
  assert.match(capture, /grid-column:\s*1 \/ -1/);

  const error = ruleOf(css, '.settings-shortcut-row .error-text') || '';
  assert.match(error, /grid-column:\s*1 \/ -1/);
});