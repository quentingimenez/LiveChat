// Vertical rhythm of the guild « Réglages » page: the server form keeps its
// distance from the hint above it, and the « Discord » sub-title from the
// save button above it.

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

const px = (declarations, property) => {
  const match = new RegExp(`${property}:\\s*(\\d+)px`).exec(declarations || '');
  return match ? Number(match[1]) : 0;
};

test('the server hint leaves room before « Nom de la guilde »', () => {
  assert.match(read('views/guild-settings.js'), /settings-hint is-lead' \}, 'Pour toute la guilde/);
  assert.ok(px(ruleOf(read('styles/settings.css'), '.settings-hint.is-lead'), 'margin-bottom') >= 16);
});

test('regression: a sub-title of a settings section keeps a top margin', () => {
  // `.settings-section h3` resets the margin of every h3 in a section and
  // wins over `.section-title` alone: the override must be as specific.
  assert.match(read('views/admin.js'), /h\('h3', \{ class: 'section-title' \}, 'Discord'\)/);
  assert.ok(px(ruleOf(read('styles/admin.css'), '.settings-section .section-title'), 'margin-top') >= 24);
});
