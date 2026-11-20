// Ligne d'occupation du board, partagée par le board et les Réglages de la
// guilde : « 10 médias · 22 Mo sur 5 Go ».
//
// La fonction ne doit jamais produire un couple taille/limite incomplet ni un
// « / » (l'ancien format, qui se lisait comme une division) : un board sans
// quota n'affiche simplement pas la ligne.

import assert from 'node:assert/strict';
import test from 'node:test';

import { formatBoardUsage } from '../../web/app/core/format.js';

const board = (itemsCount, usedBytes, maxBytes) => ({ itemsCount, usedBytes, maxBytes });

test('formatBoardUsage : compte, taille utilisée et quota', () => {
  assert.equal(formatBoardUsage(board(10, 22 * 1024 * 1024, 5 * 1024 ** 3)), '10 médias · 22 Mo sur 5 Go');
});

test('formatBoardUsage : singulier et board vide', () => {
  assert.equal(formatBoardUsage(board(1, 1024 * 1024, 1024 ** 3)), '1 média · 1 Mo sur 1 Go');
  assert.equal(formatBoardUsage(board(0, 0, 1024 ** 3)), '0 média · 0 Ko sur 1 Go');
});

test('formatBoardUsage : un compte absent vaut zéro, pas NaN', () => {
  assert.equal(formatBoardUsage(board(undefined, 1024 * 1024, 1024 ** 3)), '0 média · 1 Mo sur 1 Go');
  assert.equal(formatBoardUsage(board('4', 1024 * 1024, 1024 ** 3)), '4 médias · 1 Mo sur 1 Go');
});

test('formatBoardUsage : pas de ligne sans quota lisible', () => {
  assert.equal(formatBoardUsage(null), null);
  assert.equal(formatBoardUsage(undefined), null);
  assert.equal(formatBoardUsage({}), null);
  assert.equal(formatBoardUsage(board(3, 1024, 0)), null);
  assert.equal(formatBoardUsage(board(3, 1024, -1)), null);
  assert.equal(formatBoardUsage(board(3, 1024, Number.NaN)), null);
});

test('formatBoardUsage : les Réglages gardent la ligne sans quota', () => {
  assert.equal(formatBoardUsage(board(3, 2 * 1024 ** 3, 0), { requireMax: false }), '3 médias · 2 Go');
  assert.equal(formatBoardUsage(board(0, 0, null), { requireMax: false }), '0 média · 0 Ko');
});

test('formatBoardUsage : pas de ligne sans taille utilisée', () => {
  assert.equal(formatBoardUsage(board(3, null, 1024 ** 3)), null);
  assert.equal(formatBoardUsage(board(3, undefined, 1024 ** 3)), null);
  assert.equal(formatBoardUsage(board(3, 'beaucoup', 1024 ** 3)), null);
});

test('formatBoardUsage : le séparateur se lit « sur », jamais « / »', () => {
  const usage = formatBoardUsage(board(2, 3 * 1024 ** 3, 5 * 1024 ** 3));
  assert.match(usage, / sur /);
  assert.ok(!usage.includes('/'), usage);
  assert.match(usage, /^2 médias · 3 Go sur 5 Go$/);
});
