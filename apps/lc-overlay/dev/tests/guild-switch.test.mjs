// Switching guild from the rail while a guild tab is open (board, queue,
// administration, settings). The switch used to wait for `GET /overlay/guild`
// before painting the header: the old guild stayed on screen, clickable, under
// the new guild's identity. It now paints the rail, the header and the view at
// once from the guild list (or its cached snapshot) and revalidates in the
// background.
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

test('the switch paints the new guild before any data request', () => {
  const apply = section(data, 'export function applyGuilds', 'function resetGuildData');
  const changed = section(apply, 'if (previous !== state.activeGuildId)', '// Reconnection attempts');
  // The last snapshot (or the empty/skeleton state) is adopted first, then the
  // three zones are painted synchronously; hydration starts after that paint.
  assert.match(changed, /adoptGuild\(state\.activeGuildId\)/);
  assert.match(changed, /state\.switchingGuild = Boolean\(state\.activeGuildId\)/);
  const rail = changed.indexOf('renderRail()');
  const header = changed.indexOf('renderHeader()');
  const view = changed.indexOf('renderView()');
  const hydrate = changed.indexOf('hydrateGuild(state.activeGuildId');
  assert.ok(rail >= 0 && header > rail, 'header painted after the rail');
  assert.ok(view > header, 'view painted after the header');
  assert.ok(hydrate > view, 'hydration starts after the first paint');
  assert.doesNotMatch(changed, /\bawait\b/, 'painting must not await the server');
});

test('the duplicate switch frame repaints instead of returning early', () => {
  const apply = section(data, 'export function applyGuilds', 'function resetGuildData');
  // The shell announces one switch twice (event, then command answer). The
  // second pass must not bail out on `switchingGuild`, otherwise the header can
  // stay one frame behind; it simply keeps the view (and its scroll) as is.
  assert.doesNotMatch(apply, /if \(state\.switchingGuild\) \{\s+return;/);
  assert.match(apply, /renderHeader\(\);\s+if \(!state\.activeGuildId \|\| state\.page === 'settings'\) \{/);
});

test('a guild switch hydrates the independent resources in parallel', () => {
  const hydrate = section(data, 'export async function hydrateGuild', 'export async function loadGuild');
  assert.match(hydrate, /Promise\.allSettled\(\[/);
  for (const loader of ['loadGuildInfo', 'loadItems', 'loadQueue']) {
    assert.match(hydrate, new RegExp(`${loader}\\(\\{`), `${loader} must be in the batch`);
  }
  assert.match(hydrate, /loadBindings\(guildId\)/);
  assert.match(hydrate, /await loadAdmin\(\{ quiet \}\)/);
  assert.match(hydrate, /state\.switchingGuild = false/);
});

test('loadGuild only delegates to hydration', () => {
  const load = section(data, 'export async function loadGuild', 'export function loadItems');
  assert.match(load, /hydrateGuild\(guildId, options\)/);
  assert.doesNotMatch(load, /tabAfterGuildSwitch|renderHeader|paint: !switched/);
});

test('the tab is fixed from the real permissions once the info lands', () => {
  const info = section(data, 'async function loadGuildInfoForGuild', 'export async function hydrateGuild');
  assert.match(info, /state\.tab === 'admin' && !canAdmin\(\)/);
  assert.match(info, /state\.tab = 'board'/);
  assert.match(info, /renderHeader\(\)/);
  assert.doesNotMatch(info, /paint: false/);
});

test('a failed board read is an error, not an empty board', () => {
  const items = section(data, 'async function loadItemsForGuild', '// Signed media URLs expire');
  assert.match(items, /state\.itemsLoaded = true/);
  assert.match(items, /state\.itemsError = error\.message/);
  // Only a forbidden board is legitimately empty and cached as such.
  assert.match(items, /error\.code === 'forbidden'[\s\S]{0,200}writeGuildCache\(guildId, 'items', \[\]\)/);
});

test('a failed queue read is an error, not an empty queue', () => {
  const queue = section(data, 'async function loadQueueForGuild', 'export function loadAdmin');
  assert.match(queue, /state\.queueLoaded = true/);
  assert.match(queue, /state\.queueError = error\.message/);
  // A history read that lost the race must not overwrite the newer snapshot.
  assert.match(queue, /isLatest\(token\) && guildId === state\.activeGuildId/);
});

test('losing the active guild starts on the board', () => {
  const apply = section(data, 'export function applyGuilds', 'function resetGuildData');
  assert.match(apply, /const admin = state\.info \? canAdmin\(\) : Boolean\(/);
  assert.match(apply, /state\.tab = tabAfterGuildSwitch\(state\.tab, admin\)/);
  assert.equal(tabs.tabAfterGuildSwitch('admin', false), 'board');
});

test('the device settings event does not repaint during a guild switch', () => {
  const settings = read('views/settings.js');
  const handler = section(settings, "listen('overlay://device-settings'", "listen('overlay://navigate'");
  assert.match(handler, /!state\.switchingGuild/);
  // It still repaints for a device change made elsewhere.
  assert.match(handler, /renderView\(\)/);
});

test('the queue badge disappears until the new queue is loaded', () => {
  const header = read('views/header.js');
  assert.match(header, /const queueCount = state\.queueLoaded \? state\.queue\.pending\.length : 0;/);
  assert.doesNotMatch(header, /queueBadgeCount|lastQueueCount/);
});

test('the header trusts the role until the permissions are known', () => {
  const header = read('views/header.js');
  assert.match(header, /const canManage = state\.info \? canAdmin\(\) : role === 'admin';/);
  assert.match(header, /if \(canManage\) \{/);
});

test('the rail marks the target guild before the shell answers', () => {
  const rail = read('views/rail.js');
  assert.match(rail, /state\.pendingGuildId = guildId;/);
  assert.match(rail, /prefetchGuildInfo\(guildId\)/);
  // The command runs after the first paint, and the answer is applied only if
  // no newer click superseded it.
  const pending = rail.indexOf('state.pendingGuildId = guildId;');
  const invoke = rail.indexOf("invoke('guild_select'");
  assert.ok(pending >= 0 && invoke > pending, 'the rail paints before the command');
  assert.match(rail, /attempt === selectionAttempt/);
});
