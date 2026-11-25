// The media detail window puts « Légende » and « Raccourci » in the same facts
// grid. The caption button fills the value column. For the shortcut, the whole
// framed key must be the edit button: the key text and the pencil sit inside a
// single frame, and the clear cross sits outside it, to the right. The old
// layout shrank the key to its content and overlaid the cross inside the frame.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const css = readFileSync(new URL('../../web/app/styles/board.css', import.meta.url), 'utf8');
const detailCss = readFileSync(new URL('../../web/app/styles/media-detail.css', import.meta.url), 'utf8');
const view = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8');

function ruleIn(source, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(source)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  return '';
}

function ruleOf(selector) {
  return ruleIn(css, selector);
}

test('the shortcut cell lays the key and the clear button side by side', () => {
  const cell = ruleOf('.shortcut-cell');
  assert.match(cell, /display:\s*flex/);
  assert.match(cell, /gap:\s*6px/);
});

test('the shortcut key fills the value column like the caption button', () => {
  const caption = ruleOf('.inline-edit');
  const key = ruleOf('.shortcut-cell .shortcut-key');
  assert.match(caption, /width:\s*100%/);
  assert.match(key, /flex:\s*1(?:\s+1\s+auto)?\b/);
  assert.match(key, /min-width:\s*0/);
});

test('the assigned key is a single 32 px frame', () => {
  const key = ruleOf('.shortcut-cell .shortcut-key:has(+ .shortcut-clear)');
  assert.match(key, /min-height:\s*32px/);
  assert.match(key, /padding:\s*4px\s+8px/);
  assert.match(key, /border:\s*1px\s+solid\s+var\(--line\)/);
  assert.match(key, /justify-content:\s*space-between/);
  assert.match(key, /background:/);
});

test('the assigned key does not draw a second frame around the key text', () => {
  const kbd = ruleOf('.shortcut-cell .shortcut-key:has(+ .shortcut-clear) kbd');
  assert.match(kbd, /padding:\s*0/);
  assert.match(kbd, /border:\s*0/);
  assert.match(kbd, /background:\s*none/);
  assert.match(kbd, /font:\s*inherit/);
});

test('the clear cross sits outside the frame', () => {
  const clear = ruleOf('.shortcut-cell .shortcut-clear.btn');
  assert.match(clear, /position:\s*static/);
  assert.match(clear, /flex:\s*none/);
  assert.doesNotMatch(clear, /position:\s*absolute/);
});

test('the whole key is the edit button and the cross stays a sibling', () => {
  assert.match(view, /class: 'shortcut-key'[\s\S]{0,180}onClick: \(\) => startCapture\(item\)/);
  assert.match(view, /class: 'shortcut-key is-empty'[\s\S]{0,160}'Définir'/);
  assert.match(view, /class: 'btn is-quiet shortcut-clear'/);
});

test('the shortcut row holds only the key, without the AltGr hint', () => {
  // The AltGr explanation stays in the settings page, not in the detail window.
  assert.doesNotMatch(view, /ALTGR_HINT/);
  assert.doesNotMatch(view, /shortcut-hint/);
  assert.doesNotMatch(css, /\.shortcut-hint\b/, 'no rule left for a removed element');
  const cell = ruleOf('.shortcut-cell');
  assert.match(cell, /display:\s*flex/);
});

test('the information column scrolls on its own, so the stage stays in place', () => {
  const page = ruleIn(detailCss, '.hp-page');
  assert.match(page, /overflow:\s*hidden/);
  const info = ruleIn(detailCss, '.hp-page-info');
  assert.match(info, /overflow-y:\s*auto/);
  assert.match(info, /scrollbar-gutter:\s*stable/);
});

test('the detail window keeps the capture hint on one line', () => {
  const capturing = ruleOf('.shortcut-key.is-capturing');
  assert.match(capturing, /white-space:\s*nowrap/);
  assert.match(capturing, /font-size:\s*12px/);
  assert.match(capturing, /padding-inline:\s*4px/);
});
