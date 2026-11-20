// Protège la bascule du bouton de sélection du board : il ne propose de vider
// la sélection que lorsque toutes les cartes affichées sont déjà cochées.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { selectAllToggle } from '../../web/app/core/selection.js';

test('propose « Tout sélectionner » quand une carte affichée manque', () => {
  const result = selectAllToggle({
    orderedIds: ['a', 'b', 'c'],
    selection: new Set(['a', 'c']),
  });
  assert.deepEqual(result, { label: 'Tout sélectionner', selectAll: true });
});

test('propose « Tout désélectionner » quand toutes les cartes affichées sont cochées', () => {
  const result = selectAllToggle({
    orderedIds: ['a', 'b'],
    selection: new Set(['a', 'b', 'hors-board']),
  });
  assert.deepEqual(result, { label: 'Tout désélectionner', selectAll: false });
});

test('ignore les cartes cochées hors du board affiché', () => {
  const result = selectAllToggle({
    orderedIds: ['a'],
    selection: new Set(['a', 'hors-board']),
  });
  assert.equal(result.selectAll, false);
  assert.equal(result.label, 'Tout désélectionner');
});

test('ne propose pas de vider la sélection quand aucune carte n’est affichée', () => {
  const result = selectAllToggle({
    orderedIds: [],
    selection: new Set(['a']),
  });
  assert.deepEqual(result, { label: 'Tout sélectionner', selectAll: true });
});

test('ne modifie pas la sélection reçue', () => {
  const selection = new Set(['a']);
  selectAllToggle({ orderedIds: ['a', 'b'], selection });
  assert.deepEqual([...selection], ['a']);
});
