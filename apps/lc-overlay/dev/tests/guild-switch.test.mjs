// Switching guild from the rail while a guild tab is open (board, queue,
// administration, settings). The header used to repaint before the new guild's
// info arrived, so « Administration » appeared a frame late and the overlay
// count went from a value to nothing to a value; the administration tab was
// forced back to the board, and « Réglages » kept the previous guild's settings
// until the next click (the shell's device-settings event painted before the
// guild list).
//
// The overlay views need a DOM and the dev harness has none: the tab rule is
// imported, and the ordering rules are pinned on the source that encodes them.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import * as stateModule from '../../web/app/core/state.js';
import * as tabs from '../../web/app/core/tabs.js';

const { state } = stateModule;

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const data = read('data.js');

function section(source, from, to) {
  const start = source.indexOf(from);
  assert.ok(start >= 0, `${from} not found`);
  const end = source.indexOf(to, start);
  assert.ok(end > start, `${to} not found after ${from}`);
  return source.slice(start, end);
}

test('changing guild keeps « Administration » when the new guild grants it', () => {
  assert.equal(tabs.tabAfterGuildSwitch('admin', true), 'admin');
});

test('changing to a guild without administration rights falls back to the board', () => {
  // The rail also lists guilds where you are only a member: the tab must not
  // stay selected over a panel whose subtabs are all forbidden.
  assert.equal(tabs.tabAfterGuildSwitch('admin', false), 'board');
});

test('the board, queue and settings tabs stay selected across a guild change', () => {
  for (const tab of ['board', 'queue', 'settings']) {
    assert.equal(tabs.tabAfterGuildSwitch(tab, false), tab);
    assert.equal(tabs.tabAfterGuildSwitch(tab, true), tab);
  }
});

test('any one of the three administration rights opens the tab', () => {
  const saved = state.info;
  try {
    for (const permission of ['manageMembers', 'manageInvites', 'manageSettings']) {
      state.info = { permissions: { [permission]: true } };
      assert.equal(stateModule.canAdmin(), true);
    }
    state.info = { permissions: { viewBoard: true, play: true } };
    assert.equal(stateModule.canAdmin(), false);
    state.info = null;
    assert.equal(stateModule.canAdmin(), false);
  } finally {
    state.info = saved;
  }
});

test('a guild change no longer resets « Administration » to the board', () => {
  const reset = section(data, 'function resetGuildData', 'export function guildInfoAge');
  assert.doesNotMatch(reset, /state\.tab/);
});

test('the header waits for the switched guild info', () => {
  const apply = section(data, 'export function applyGuilds', 'function resetGuildData');
  const changed = section(apply, 'if (previous !== state.activeGuildId)', 'if (state.activeGuildId) {');
  // The switch paints the rail at once, and nothing else until the info lands:
  // rendering the header now would hide the tab and the count for a frame.
  assert.match(changed, /state\.switchingGuild = true/);
  assert.match(changed, /loadGuild\(\{ switched: true \}\)/);
  assert.doesNotMatch(changed, /renderHeader|renderView/);
  // The duplicate frame (shell event then command answer) must not repaint
  // with the info still loading.
  assert.match(apply, /if \(state\.switchingGuild\) \{\s+return;\s+\}\s+renderHeader\(\);/);
});

test('the switched guild repaints « Réglages » once its info is known', () => {
  const load = section(data, 'export async function loadGuild(', 'export async function loadItems');
  assert.match(load, /state\.tab = tabAfterGuildSwitch\(state\.tab, canAdmin\(\)\);/);
  assert.match(load, /state\.tab === 'settings'[\s\S]{0,400}renderView\(\)/);
});

test('losing the guild starts the next one on the board', () => {
  // « Administration » chosen for a removed guild must not land the next guild
  // (joined later, possibly member-only) on the administration panel.
  const load = section(data, 'export async function loadGuild(', 'export async function loadItems');
  assert.match(load, /if \(!state\.activeGuildId\) \{[\s\S]{0,400}state\.tab = 'board';/);
});

test('the device settings event does not repaint during a guild switch', () => {
  const settings = read('views/settings.js');
  const handler = section(settings, "listen('overlay://device-settings'", "listen('overlay://navigate'");
  assert.match(handler, /!state\.switchingGuild/);
  // It still repaints for a device change made elsewhere.
  assert.match(handler, /renderView\(\)/);
});
