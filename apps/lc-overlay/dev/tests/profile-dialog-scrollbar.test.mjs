// Guild profile dialog: the frame is taller than the board at its minimum size
// (960×640). Its body must keep scrolling without drawing a scrollbar over the
// fixed dialog layout.
//
// No DOM in the overlay test dependencies: the class contract is a pure helper
// and the CSS is checked as source.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { dialogClassNames } from '../../web/app/core/ui.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

// Declarations of the first rule whose selector list contains `selector`.
function declarationsOf(css, selector) {
  const rule = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rule.exec(css)) !== null) {
    const selectors = match[1]
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .split(',')
      .map((part) => part.trim());
    if (selectors.includes(selector)) {
      return match[2];
    }
  }
  assert.fail(`règle ${selector} introuvable`);
}

test('dialogClassNames garde le cadre commun et ajoute la classe demandée', () => {
  assert.equal(dialogClassNames(), 'dialog');
  assert.equal(dialogClassNames({}), 'dialog');
  assert.equal(dialogClassNames({ width: 'wide' }), 'dialog is-wide');
  assert.equal(
    dialogClassNames({ width: 'wide', dialogClass: 'is-profile' }),
    'dialog is-wide is-profile',
  );
  assert.equal(dialogClassNames({ dialogClass: '  is-profile  ' }), 'dialog is-profile');
  assert.equal(dialogClassNames({ dialogClass: '   ' }), 'dialog');
});

test('le dialogue de profil est le seul à porter is-profile', () => {
  const ui = read('core/ui.js');
  const profile = read('views/profile.js');
  assert.match(ui, /class: dialogClassNames\(options\)/);
  assert.match(profile, /dialogClass: 'is-profile'/);
});

test('le corps du profil défile toujours, sans barre visible', () => {
  const css = read('styles/dialog.css');
  const body = declarationsOf(css, '.dialog-body');
  assert.match(body, /overflow-y:\s*auto/);

  const profileBody = declarationsOf(css, '.dialog.is-profile .dialog-body');
  assert.match(profileBody, /overflow-y:\s*auto/);
  assert.match(profileBody, /scrollbar-width:\s*none/);
  assert.match(profileBody, /-ms-overflow-style:\s*none/);
  assert.doesNotMatch(profileBody, /overflow(?:-y)?:\s*hidden/);

  const webkit = declarationsOf(css, '.dialog.is-profile .dialog-body::-webkit-scrollbar');
  assert.match(webkit, /display:\s*none/);
});
