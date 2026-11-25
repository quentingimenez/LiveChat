// Protège l'état actif des chips de type du board : la feuille de style peint
// `.is-active` ET `[aria-pressed="true"]`, donc un clic doit éteindre les deux
// marqueurs de l'ancien type. Avant le correctif, seul `is-active` suivait le
// clic : l'ancien chip gardait `aria-pressed="true"` et restait allumé à côté
// du nouveau jusqu'au prochain rendu complet (le bouton « Filtres » en
// déclenchait un par accident).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { syncKindChips } from '../../web/app/core/kind-chips.js';

function fakeChip(kind, { active = false, pressed = active } = {}) {
  const classes = new Set(active ? ['is-active'] : []);
  const attributes = new Map([['aria-pressed', String(pressed)]]);
  return {
    dataset: { kind },
    classList: {
      toggle(name, on) {
        if (on) {
          classes.add(name);
        } else {
          classes.delete(name);
        }
      },
      contains(name) {
        return classes.has(name);
      },
    },
    setAttribute(name, value) {
      attributes.set(name, value);
    },
    getAttribute(name) {
      return attributes.has(name) ? attributes.get(name) : null;
    },
  };
}

const stateOf = (chip) => ({
  kind: chip.dataset.kind,
  active: chip.classList.contains('is-active'),
  pressed: chip.getAttribute('aria-pressed'),
});

const chipsFor = (selected) => [
  fakeChip('all', { active: selected === 'all' }),
  fakeChip('video', { active: selected === 'video' }),
  fakeChip('image', { active: selected === 'image' }),
  fakeChip('audio', { active: selected === 'audio' }),
];

test('un clic éteint les deux marqueurs de l’ancien type', () => {
  const chips = chipsFor('all');
  syncKindChips(chips, 'video');
  assert.deepEqual(chips.map(stateOf), [
    { kind: 'all', active: false, pressed: 'false' },
    { kind: 'video', active: true, pressed: 'true' },
    { kind: 'image', active: false, pressed: 'false' },
    { kind: 'audio', active: false, pressed: 'false' },
  ]);
});

test('le chip sélectionné répare un rendu incohérent', () => {
  // L'état exact du bug : l'ancien type est encore pressé, le nouveau n'a que
  // la classe, et la feuille de style allume donc les deux.
  const chips = [fakeChip('all', { active: false, pressed: true }), fakeChip('video', { active: true, pressed: false })];
  syncKindChips(chips, 'video');
  assert.deepEqual(chips.map(stateOf), [
    { kind: 'all', active: false, pressed: 'false' },
    { kind: 'video', active: true, pressed: 'true' },
  ]);
});

test('quel que soit le clic, un seul type est actif et pressé', () => {
  const chips = chipsFor('all');
  for (const selected of ['video', 'image', 'audio', 'all', 'video']) {
    syncKindChips(chips, selected);
    assert.deepEqual(
      chips.filter((chip) => chip.classList.contains('is-active')).map((chip) => chip.dataset.kind),
      [selected],
    );
    for (const chip of chips) {
      assert.equal(
        chip.getAttribute('aria-pressed'),
        String(chip.dataset.kind === selected),
        `${chip.dataset.kind}: aria-pressed doit suivre la sélection`,
      );
    }
  }
});

test('setKind met à jour les chips par la même fonction', () => {
  const board = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8');
  const setKind = /function setKind\(kind\) \{[\s\S]*?\n\}/.exec(board)?.[0] ?? '';
  assert.ok(setKind, 'setKind doit exister');
  assert.match(
    setKind,
    /syncKindChips\(document\.querySelectorAll\('\.board-toolbar \.chip'\), kind\)/,
    'setKind doit synchroniser les deux marqueurs des chips de la barre',
  );
});
