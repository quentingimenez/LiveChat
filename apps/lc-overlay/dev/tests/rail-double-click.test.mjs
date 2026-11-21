// Double-clic sur une guilde du rail : il ouvre la board de cette guilde, quel
// que soit l'onglet ouvert (file d'attente, administration, réglages de la
// guilde, paramètres de l'appareil). Le simple clic ne change pas ce
// comportement : il sélectionne la guilde et garde l'onglet ouvert.
//
// La règle de navigation vit dans core/rail.js, sans DOM. Le câblage du
// gestionnaire est épinglé sur views/rail.js : le harnais n'a pas de DOM, donc
// les vues ne peuvent pas être importées ici.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { openGuildBoard } from '../../web/app/core/rail.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

function guildState(overrides) {
  return {
    page: 'guild',
    tab: 'queue',
    switchingGuild: false,
    activeGuildId: 'g1',
    ...overrides,
  };
}

function section(source, from, to) {
  const start = source.indexOf(from);
  assert.ok(start >= 0, `${from} not found`);
  const end = source.indexOf(to, start);
  assert.ok(end > start, `${to} not found after ${from}`);
  return source.slice(start, end);
}

test('opens the board from every guild tab', () => {
  for (const tab of ['board', 'queue', 'admin', 'settings']) {
    const state = guildState({ tab });
    assert.equal(openGuildBoard(state, 'g1'), true, `tab ${tab} must repaint`);
    assert.equal(state.page, 'guild');
    assert.equal(state.tab, 'board');
  }
});

test('comes back from the device settings page', () => {
  const state = guildState({ page: 'settings', tab: 'queue' });
  assert.equal(openGuildBoard(state, 'g1'), true);
  assert.equal(state.page, 'guild');
  assert.equal(state.tab, 'board');
});

test('leaves the board to the switch that is already running', () => {
  // The click requested the new guild; its info is not painted yet. Painting
  // now would put the previous guild's board under the new guild's name.
  const state = guildState({ tab: 'queue', switchingGuild: true, activeGuildId: 'g2' });
  assert.equal(openGuildBoard(state, 'g2'), false);
  // The board is still the target the switch will paint.
  assert.equal(state.page, 'guild');
  assert.equal(state.tab, 'board');
});

test('waits for the selection command before painting another guild', () => {
  const state = guildState({ tab: 'admin', activeGuildId: 'g1' });
  assert.equal(openGuildBoard(state, 'g2'), false);
  assert.equal(state.page, 'guild');
  assert.equal(state.tab, 'board');
});

test('paints once the double-clicked guild is active', () => {
  const state = guildState({ tab: 'queue', activeGuildId: 'g2' });
  assert.equal(openGuildBoard(state, 'g2'), true);
  assert.equal(state.page, 'guild');
  assert.equal(state.tab, 'board');
});

test('the guild tile wires the double click to the board rule', () => {
  const rail = read('views/rail.js');
  assert.match(rail, /import \{ openGuildBoard \} from '\.\.\/core\/rail\.js';/);
  assert.match(rail, /onDblclick: \(\) => showGuildBoard\(guild\)/);
  const handler = section(rail, 'function showGuildBoard', 'export function renderRail');
  assert.match(handler, /openGuildBoard\(state, guild\.membership\.guildId\)/);
  assert.match(handler, /renderHeader\(\);\s+renderView\(\);/);
});

test('a single click still only selects the guild', () => {
  const rail = read('views/rail.js');
  assert.match(rail, /onClick: \(\) => selectGuild\(guild\.membership\.guildId\)/);
  // selectGuild selects the guild; the tab is kept across the switch by
  // tabAfterGuildSwitch. Moving it here would make the simple click lose the
  // open tab too.
  const select = rail.slice(rail.indexOf('export async function selectGuild'));
  assert.doesNotMatch(select, /state\.tab\s*=/);
});
