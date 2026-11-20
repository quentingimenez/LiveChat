// « Mon profil » → « Extension navigateur » : the created code shows two
// labelled fields, the server URL and the code, each closed by a copy button.
// The URL fills its row (`.profile-value` is `flex: 1`), so its button lands on
// the right edge; the code is only as wide as its text, so its button sat
// straight after it. Both buttons must close their row on the right.

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

test('the copy button of a profile field sits on the right edge', () => {
  const declarations = ruleOf(read('styles/dialog.css'), '.profile-dialog .profile-field-value > .profile-copy');
  assert.ok(declarations, 'no rule for the copy button of a profile field');
  assert.match(declarations, /margin-left:\s*auto/);
});

test('the code of the extension panel is a field row, like the server URL', () => {
  // The rule above only reaches the code's button while it stays a child of
  // `.profile-field-value`; a `profile-code-main` row would escape it.
  assert.match(
    read('views/profile.js'),
    /class: 'profile-field-value'[\s\S]{0,400}?copyButton\('Copier le code'/,
  );
});
