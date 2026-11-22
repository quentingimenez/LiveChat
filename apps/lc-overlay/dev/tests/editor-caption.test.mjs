// What this file protects: the caption edited in place in the board's detail
// drawer keeps its text off the field's border — the frame starts in the
// gutter so the text stays aligned with the other values.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('la légende du tiroir n’est plus collée à la bordure du champ', () => {
  const css = read('styles/board.css');
  const rule = css.match(/\.facts \.inline-edit \{[^}]*\}/);
  assert.ok(rule, 'règle .facts .inline-edit absente');
  assert.match(rule[0], /padding: 4px 8px/);
  assert.match(rule[0], /margin: 0 0 0 -8px/);
  assert.match(rule[0], /width: calc\(100% \+ 8px\)/);
  assert.doesNotMatch(rule[0], /padding: 2px 0/);
});
