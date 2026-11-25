// What this file protects: the caption edited in place in the media detail
// window keeps its text off the field's border — the frame starts in the
// gutter so the text stays aligned with the « Légende » label above it. The
// caption moved out of the facts grid into its own block; the gutter rule moved
// with it into the window stylesheet.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('la légende de la fenêtre de détail n’est plus collée à la bordure du champ', () => {
  const css = read('styles/media-detail.css');
  const rule = css.match(/\.hp-page-legend-block \.inline-edit,[^{]*\{[^}]*\}/);
  assert.ok(rule, 'règle .hp-page-legend-block .inline-edit absente');
  assert.match(rule[0], /padding: 4px 8px/);
  assert.match(rule[0], /margin: 0 0 0 -8px/);
  assert.match(rule[0], /width: calc\(100% \+ 8px\)/);
  assert.doesNotMatch(rule[0], /padding: 2px 0/);
  // The editing field keeps the same gutter as the button it replaces.
  assert.match(rule[0], /\.inline-input/);
});
