// Protège la position de lecture du board : entrer ou sortir du mode sélection
// ne doit pas ramener la vue tout en haut, et une repeinte du board garde la
// position tandis qu'un vrai changement de vue repart du haut.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { boardScrollToKeep } from '../../web/app/core/scroll-memory.js';

const boardSource = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8');
const viewSource = readFileSync(new URL('../../web/app/views/view.js', import.meta.url), 'utf8');

test('garde la position quand seule la board est repeinte', () => {
  assert.equal(boardScrollToKeep({ hadBoard: true, hasBoard: true, scrollTop: 4096 }), 4096);
});

test('laisse la main au navigateur quand la vue change', () => {
  assert.equal(boardScrollToKeep({ hadBoard: true, hasBoard: false, scrollTop: 4096 }), null);
  assert.equal(boardScrollToKeep({ hadBoard: false, hasBoard: true, scrollTop: 4096 }), null);
});

test('ignore une position invalide', () => {
  for (const scrollTop of [Number.NaN, Number.POSITIVE_INFINITY, -12, 'nope', null]) {
    assert.equal(boardScrollToKeep({ hadBoard: true, hasBoard: true, scrollTop }), 0);
  }
});

test('garde une position fractionnaire (zoom)', () => {
  assert.equal(boardScrollToKeep({ hadBoard: true, hasBoard: true, scrollTop: 12.5 }), 12.5);
});

test('renderView remet la position sauvée après avoir remplacé le contenu', () => {
  assert.match(viewSource, /const hadBoard = view\.querySelector\('\.board'\) !== null;/);
  assert.match(viewSource, /const scrollTop = view\.scrollTop;/);
  assert.match(viewSource, /const keepScroll = boardScrollToKeep\(\{[\s\S]*?\n  \}\);/);
  assert.match(viewSource, /view\.scrollTop = keepScroll;/);
});

test('entrer en sélection peint la board sur place', () => {
  const enter = functionBody('export function enterSelection()');
  assert.doesNotMatch(enter, /renderView\(/);
  assert.match(enter, /syncSelection\(\);/);
  assert.match(enter, /document\.getElementById\('grid'\)\?\.classList\.add\('is-selecting'\)/);
});

test('quitter la sélection enlève la barre sur place', () => {
  const leave = functionBody('export function leaveSelection()');
  assert.doesNotMatch(leave, /renderView\(/);
  assert.match(leave, /syncSelection\(\);/);
  assert.match(leave, /document\.getElementById\('grid'\)\?\.classList\.remove\('is-selecting'\)/);
});

test('la barre de sélection est créée quand elle manque', () => {
  const bar = functionBody('function syncSelectionBar()');
  assert.match(bar, /document\.querySelector\('\.view \.board'\)\?\.after\(next\)/);
  assert.match(boardSource, /id: 'select-toggle'/);
  const toggle = functionBody('function syncSelectionToggle()');
  assert.match(toggle, /toggle\.textContent = state\.selecting \? 'Terminer' : 'Sélectionner';/);
});

function functionBody(start) {
  const lines = boardSource.split(/\r?\n/);
  const from = lines.findIndex((line) => line.trimStart().startsWith(start));
  assert.ok(from >= 0, `fonction absente : ${start}`);
  const end = lines.indexOf('}', from);
  assert.ok(end > from, `fin de fonction absente : ${start}`);
  return lines.slice(from, end + 1).join('\n');
}
