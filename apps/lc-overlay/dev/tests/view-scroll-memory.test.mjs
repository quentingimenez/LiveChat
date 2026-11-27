// Protège la position de lecture : une repeinte de la même vue (paramètres,
// board, file, admin, réglages de guilde) garde la position ; un vrai
// changement de vue repart du haut, jamais au milieu de la page suivante.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { viewScrollToKeep } from '../../web/app/core/scroll-memory.js';

const boardSource = readFileSync(new URL('../../web/app/views/board.js', import.meta.url), 'utf8');
const viewSource = readFileSync(new URL('../../web/app/views/view.js', import.meta.url), 'utf8');

test('garde la position quand la même vue est repeinte', () => {
  assert.equal(viewScrollToKeep({ previousView: 'settings', nextView: 'settings', scrollTop: 4096 }), 4096);
  assert.equal(
    viewScrollToKeep({ previousView: 'guild:42:board', nextView: 'guild:42:board', scrollTop: 1659 }),
    1659,
  );
});

test('un changement de vue repart du haut', () => {
  const scrollTop = 4096;
  for (const [previousView, nextView] of [
    ['guild:42:board', 'settings'],
    ['settings', 'guild:42:board'],
    ['guild:1:board', 'guild:2:board'],
    ['guild:1:board', 'guild:1:admin'],
    ['guild:1:admin:members', 'guild:1:admin:devices'],
    ['join', 'guild:42:board'],
  ]) {
    assert.equal(viewScrollToKeep({ previousView, nextView, scrollTop }), 0);
  }
});

test('premier rendu : départ du haut', () => {
  assert.equal(viewScrollToKeep({ previousView: null, nextView: 'settings', scrollTop: 0 }), 0);
});

test('ignore une position invalide', () => {
  for (const scrollTop of [Number.NaN, Number.POSITIVE_INFINITY, -12, 'nope', null]) {
    assert.equal(viewScrollToKeep({ previousView: 'settings', nextView: 'settings', scrollTop }), 0);
  }
});

test('garde une position fractionnaire (zoom)', () => {
  assert.equal(viewScrollToKeep({ previousView: 'settings', nextView: 'settings', scrollTop: 12.5 }), 12.5);
});

test('renderView remet la position sauvée après avoir remplacé le contenu', () => {
  assert.match(viewSource, /let paintedView = null;/);
  assert.match(viewSource, /function shownViewKey\(\) \{/);
  assert.match(viewSource, /const previousView = paintedView;/);
  assert.match(viewSource, /const nextView = shownViewKey\(\);/);
  assert.match(viewSource, /const keepScroll = viewScrollToKeep\(\{/);
  assert.match(viewSource, /view\.scrollTop = keepScroll;/);
  assert.match(viewSource, /paintedView = nextView;/);
});

test('l’identité de vue couvre les paramètres, join et la section d’administration', () => {
  assert.ok(viewSource.includes("return 'settings';"));
  assert.ok(viewSource.includes("return 'join';"));
  assert.ok(
    viewSource.includes("const section = state.tab === 'admin' ? state.adminTab || '' : '';"),
    'la section d’administration fait partie de la vue',
  );
  assert.ok(
    viewSource.includes('return `guild:${state.activeGuildId}:${state.tab}:${section}`;'),
    'la clé de vue couvre la guilde, l’onglet et la section',
  );
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
