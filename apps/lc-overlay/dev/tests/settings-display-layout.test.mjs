// Display settings layout: the screen, zone and alignment rows share one
// field column and one 145px action column, and the zone and alignment rows
// sit in one framed group. These are the invariants pinned by the 1300px
// mock: the fields end on the same vertical line and the actions do too.

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

test('display rows use one field column and one action column', () => {
  const control = ruleOf(css, '.settings-display-row .settings-control') || '';
  assert.match(control, /display:\s*grid/);
  assert.match(control, /grid-template-columns:\s*minmax\(0,\s*1fr\)\s+145px/);
  assert.match(control, /column-gap:\s*10px/);
});

test('screen, zone and alignment fills share the field column', () => {
  const field = ruleOf(css, '.settings-display-row .settings-control > .field') || '';
  assert.match(field, /width:\s*100%/);
  assert.match(field, /max-width:\s*100%/);

  const zone = ruleOf(css, '.settings-display-row .settings-zone') || '';
  assert.match(zone, /width:\s*100%/);
  assert.match(zone, /height:\s*var\(--control-h\)/);
});

test('the zone and alignment rows are wrapped in one framed group', () => {
  assert.match(view, /h\('div', \{ class: 'settings-zone-group' \}, zoneRow, alignmentRow\)/);
  assert.equal((view.match(/classList\.add\('settings-display-row'\)/g) || []).length, 3);

  const group = ruleOf(css, '.settings-zone-group') || '';
  assert.match(group, /margin:\s*2px -12px 6px/);
  assert.match(group, /padding:\s*4px 11px/);
  assert.match(group, /border:\s*1px solid var\(--line\)/);
  assert.match(group, /border-radius:\s*var\(--radius-m\)/);
});

test('zone actions stack in the action column', () => {
  const actions = ruleOf(css, '.settings-zone-actions') || '';
  assert.match(actions, /display:\s*flex/);
  assert.match(actions, /flex-direction:\s*column/);
});

test('display buttons centre their label', () => {
  const button = ruleOf(css, '.settings-display-row .btn') || '';
  assert.match(button, /justify-content:\s*center/);
  assert.match(button, /text-align:\s*center/);
});
