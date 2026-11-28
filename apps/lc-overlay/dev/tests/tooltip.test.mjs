// What this file protects: the hover labels.
//
// The board moved its tooltips from the native `title` attribute to its own
// layer. The text split decides whether a shortcut becomes a chip, the
// placement keeps the tip over the trigger and inside the window, and the
// adoption removes every title while keeping an accessible description. A
// trigger that never had a title must be left untouched.

import assert from 'node:assert/strict';
import test from 'node:test';

import { adoptTitle, adoptTitles, placeTip, splitShortcut } from '../../web/app/core/tooltip.js';

const WINDOW = { width: 800, height: 600 };

test('a trailing shortcut leaves the label alone and becomes the key', () => {
  assert.deepEqual(splitShortcut('Annuler (Ctrl+Z)'), { label: 'Annuler', key: 'Ctrl+Z' });
  assert.deepEqual(splitShortcut('Fermer l’éditeur (Échap)'), {
    label: 'Fermer l’éditeur',
    key: 'Échap',
  });
  assert.deepEqual(splitShortcut('Ne garder que la sélection (Entrée)'), {
    label: 'Ne garder que la sélection',
    key: 'Entrée',
  });
  assert.deepEqual(splitShortcut('Changer le raccourci (Ctrl + Maj + 1)'), {
    label: 'Changer le raccourci',
    key: 'Ctrl + Maj + 1',
  });
});

test('a parenthetical sentence is not a shortcut', () => {
  const zoom = 'Ajuster l’aperçu (Ctrl + molette pour zoomer)';
  assert.deepEqual(splitShortcut(zoom), { label: zoom, key: '' });
  assert.deepEqual(splitShortcut('Un titre (2024)'), { label: 'Un titre (2024)', key: '' });
  assert.deepEqual(splitShortcut('Voir (https://example.com)'), {
    label: 'Voir (https://example.com)',
    key: '',
  });
});

test('plain labels stay whole and a bare key is not a label', () => {
  assert.deepEqual(splitShortcut('Actualiser'), { label: 'Actualiser', key: '' });
  assert.deepEqual(splitShortcut('  Source  '), { label: 'Source', key: '' });
  // Splitting « (Ctrl+Z) » alone would leave an empty label: keep the text.
  assert.deepEqual(splitShortcut('(Ctrl+Z)'), { label: '(Ctrl+Z)', key: '' });
});

test('the tip sits centred above its trigger', () => {
  assert.deepEqual(
    placeTip({ left: 100, top: 200, width: 100, height: 30 }, { width: 80, height: 28 }, WINDOW),
    { left: 110, top: 164 },
  );
});

test('a trigger at the top flips the tip below', () => {
  assert.deepEqual(
    placeTip({ left: 100, top: 20, width: 100, height: 30 }, { width: 80, height: 28 }, WINDOW),
    { left: 110, top: 58 },
  );
});

test('the tip stays inside the window', () => {
  const placed = placeTip(
    { left: 780, top: 300, width: 10, height: 30 },
    { width: 80, height: 28 },
    WINDOW,
  );
  assert.deepEqual(placed, { left: 712, top: 264 });
  const left = placeTip({ left: 0, top: 300, width: 10, height: 30 }, { width: 80, height: 28 }, WINDOW);
  assert.equal(left.left, 8);
});

test('a window shorter than the tip keeps the top margin', () => {
  assert.deepEqual(
    placeTip({ left: 0, top: 0, width: 10, height: 10 }, { width: 80, height: 40 }, {
      width: 800,
      height: 50,
    }),
    { left: 8, top: 8 },
  );
});

// A minimal element: the tests need attributes, textContent and nothing else.
function fakeElement(attributes = {}, text = '') {
  const values = new Map(Object.entries(attributes));
  return {
    nodeType: 1,
    getAttribute: (name) => (values.has(name) ? values.get(name) : null),
    setAttribute: (name, value) => values.set(name, String(value)),
    removeAttribute: (name) => values.delete(name),
    hasAttribute: (name) => values.has(name),
    get textContent() {
      return text;
    },
  };
}

test('an icon-only trigger gets the tooltip text as its accessible name', () => {
  const button = fakeElement({ title: 'Actualiser' });
  assert.equal(adoptTitle(button), true);
  assert.equal(button.getAttribute('title'), null);
  assert.equal(button.getAttribute('data-tip'), 'Actualiser');
  assert.equal(button.getAttribute('aria-label'), 'Actualiser');
  assert.equal(button.getAttribute('aria-description'), null);
});

test('a trigger with visible text keeps that name and gains a description', () => {
  const button = fakeElement({ title: 'Lire un texte à voix haute sur les overlays' }, 'Lire…');
  assert.equal(adoptTitle(button), true);
  assert.equal(button.getAttribute('aria-label'), null);
  assert.equal(button.getAttribute('aria-description'), 'Lire un texte à voix haute sur les overlays');
});

test('an existing accessible name is not overwritten', () => {
  const renamed = fakeElement({ title: 'Actualiser', 'aria-label': 'Rafraîchir' });
  adoptTitle(renamed);
  assert.equal(renamed.getAttribute('aria-label'), 'Rafraîchir');
  assert.equal(renamed.getAttribute('aria-description'), 'Actualiser');

  const same = fakeElement({ title: 'Actualiser', 'aria-label': 'Actualiser' });
  adoptTitle(same);
  assert.equal(same.getAttribute('aria-description'), null);

  const described = fakeElement({ title: 'Actualiser', 'aria-describedby': 'hint' }, 'Lire…');
  adoptTitle(described);
  assert.equal(described.getAttribute('aria-description'), null);
});

test('the shortcut stays in the accessible name', () => {
  const undo = fakeElement({ title: 'Annuler (Ctrl+Z)' });
  adoptTitle(undo);
  assert.equal(undo.getAttribute('data-tip'), 'Annuler (Ctrl+Z)');
  assert.equal(undo.getAttribute('aria-label'), 'Annuler (Ctrl+Z)');
});

test('a trigger without a title is left untouched', () => {
  const element = fakeElement({}, 'Texte');
  assert.equal(adoptTitle(element), false);
  assert.equal(element.getAttribute('data-tip'), null);
  assert.equal(element.getAttribute('aria-label'), null);
});

test('an empty title is removed without creating a tooltip', () => {
  const element = fakeElement({ title: '   ' });
  assert.equal(adoptTitle(element), false);
  assert.equal(element.getAttribute('title'), null);
  assert.equal(element.getAttribute('data-tip'), null);
});

test('a node added with its own title is adopted before its subtree', () => {
  const child = fakeElement({ title: 'Enfant' });
  const root = fakeElement({ title: 'Racine' });
  root.matches = (selector) => selector === '[title]' && root.hasAttribute('title');
  root.querySelectorAll = () => [child];
  assert.equal(adoptTitles(root), 2);
  assert.equal(root.getAttribute('data-tip'), 'Racine');
  assert.equal(child.getAttribute('data-tip'), 'Enfant');
});
