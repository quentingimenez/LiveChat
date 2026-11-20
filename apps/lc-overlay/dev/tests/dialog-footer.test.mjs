// The dialog footer lives outside the scrolling body. A submit button taken out
// of its form must keep its form owner, or clicking it does nothing and
// `form.requestSubmit(button)` throws. This guards that re-association.

import assert from 'node:assert/strict';
import test from 'node:test';

import { associateSubmitButtons } from '../../web/app/core/ui.js';

// Minimal stand-ins: only the members the helper touches.
function fakeActions(count) {
  const buttons = Array.from({ length: count }, () => {
    const attributes = new Map();
    return { attributes, setAttribute: (name, value) => attributes.set(name, value) };
  });
  return { buttons, querySelectorAll: () => buttons };
}

test('les boutons du pied retrouvent leur formulaire', () => {
  const form = { id: '' };
  const actions = fakeActions(2);
  associateSubmitButtons(actions, form);
  assert.match(form.id, /^dialog-form-/);
  for (const button of actions.buttons) {
    assert.equal(button.attributes.get('form'), form.id);
  }
});

test('un formulaire déjà nommé est réutilisé', () => {
  const form = { id: 'media-form' };
  const actions = fakeActions(1);
  associateSubmitButtons(actions, form);
  assert.equal(form.id, 'media-form');
  assert.equal(actions.buttons[0].attributes.get('form'), 'media-form');
});

test('sans formulaire, aucun bouton n’est touché', () => {
  const actions = fakeActions(1);
  associateSubmitButtons(actions, null);
  assert.equal(actions.buttons[0].attributes.size, 0);
});
