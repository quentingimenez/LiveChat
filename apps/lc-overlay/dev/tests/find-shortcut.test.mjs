// Protège le raccourci Ctrl+F de la board : il réserve la touche au champ de
// recherche de la board (focus), ou l'avale quand ce champ n'est pas affiché,
// au lieu de laisser WebView2 ouvrir sa barre « Rechercher dans la page ».

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { findAction } from '../../web/app/core/find-shortcut.js';

function findState(overrides = {}) {
  return {
    key: 'f',
    ctrl: true,
    meta: false,
    altKey: false,
    handled: false,
    capturing: false,
    dialogOpen: false,
    searchAvailable: true,
    ...overrides,
  };
}

test('Ctrl+F donne le focus au champ de recherche de la board', () => {
  assert.equal(findAction(findState()), 'focusSearch');
});

test('Cmd+F (macOS) donne le même focus', () => {
  assert.equal(findAction(findState({ ctrl: false, meta: true })), 'focusSearch');
});

test('reconnaît la touche majuscule (verrouillage majuscules)', () => {
  assert.equal(findAction(findState({ key: 'F' })), 'focusSearch');
});

test('ignore f sans Ctrl ni Cmd', () => {
  assert.equal(findAction(findState({ ctrl: false })), null);
});

test('ignore Alt+Ctrl+F (AltGr)', () => {
  assert.equal(findAction(findState({ altKey: true })), null);
});

test('ignore une touche déjà consommée par le menu contextuel', () => {
  assert.equal(findAction(findState({ handled: true })), null);
});

test('ignore Ctrl+F pendant la capture d’un raccourci média', () => {
  assert.equal(findAction(findState({ capturing: true })), null);
});

test('avale Ctrl+F quand une boîte de dialogue est ouverte, sans voler son focus', () => {
  assert.equal(findAction(findState({ dialogOpen: true })), 'swallow');
});

test('avale Ctrl+F sans champ de recherche, pour ne pas ouvrir la barre du navigateur', () => {
  assert.equal(findAction(findState({ searchAvailable: false })), 'swallow');
});

test('ignore les autres touches', () => {
  assert.equal(findAction(findState({ key: 'a' })), null);
  assert.equal(findAction(findState({ key: 'F3' })), null);
});
