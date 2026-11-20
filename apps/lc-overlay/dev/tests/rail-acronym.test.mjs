// Guild-tile fallback: the rail shows Discord's acronym (one letter per word,
// case kept) inside the tile, and clips it there. The status dot sits outside
// the tile, so the button itself must never clip.

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

const rail = () => read('views/rail.js');
const railCss = () => read('styles/rail.css');

test('the guild tile falls back to the Discord acronym, not to initials', () => {
  assert.match(rail(), /guildAcronym\(label\)/);
  assert.match(rail(), /class: 'guild-acronym'/);
  assert.doesNotMatch(rail(), /initials\(/);
});

test('a long acronym is clipped inside the tile, the dot outside it', () => {
  const acronym = ruleOf(railCss(), '.guild .guild-acronym') || '';
  assert.match(acronym, /overflow:\s*hidden/);
  assert.match(acronym, /white-space:\s*nowrap/);
  assert.match(acronym, /width:\s*100%/);
  // The tile has no inner padding left by the browser: the acronym spans it.
  assert.match(ruleOf(railCss(), '.guild') || '', /padding:\s*0/);
  // The status dot is positioned outside the button: clipping it would cut it.
  assert.doesNotMatch(ruleOf(railCss(), '.guild') || '', /overflow:\s*hidden/);
});
