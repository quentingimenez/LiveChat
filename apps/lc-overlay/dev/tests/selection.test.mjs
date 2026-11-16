import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  batches,
  cardClickAction,
  escapeAction,
  importSummary,
  importTargets,
  menuTargetIds,
  pruneSelection,
  selectRange,
  toggleId,
} from '../../web/app/core/selection.js';

const sorted = (set) => [...set].sort();

test('toggleId adds then removes', () => {
  const once = toggleId(new Set(), 'a');
  assert.deepEqual(sorted(once), ['a']);
  assert.deepEqual(sorted(toggleId(once, 'a')), []);
  assert.deepEqual(sorted(once), ['a'], 'the input set is not changed');
});

test('selectRange selects between anchor and id both ways', () => {
  const order = ['a', 'b', 'c', 'd', 'e'];
  assert.deepEqual(sorted(selectRange(new Set(['a']), order, 'b', 'd')), ['a', 'b', 'c', 'd']);
  assert.deepEqual(sorted(selectRange(new Set(), order, 'd', 'b')), ['b', 'c', 'd']);
});

test('selectRange without anchor adds the id', () => {
  const order = ['a', 'b', 'c'];
  assert.deepEqual(sorted(selectRange(new Set(), order, null, 'b')), ['b']);
  assert.deepEqual(sorted(selectRange(new Set(), order, 'hidden', 'c')), ['c']);
});

test('pruneSelection drops ids no longer on the board', () => {
  assert.deepEqual(sorted(pruneSelection(new Set(['a', 'gone', 'c']), ['a', 'b', 'c'])), ['a', 'c']);
});

const guild = (guildId, serverUrl, extra = {}) => ({
  membership: { guildId, serverUrl, guildName: guildId, role: 'member', guestMode: false, ...extra },
});

test('importTargets keeps other guilds on the same server only', () => {
  const guilds = [
    guild('a', 'https://lc.example.test/'),
    guild('b', 'https://LC.example.test'),
    guild('c', 'https://other.test'),
  ];
  assert.deepEqual(
    importTargets(guilds, 'a').map((entry) => entry.membership.guildId),
    ['b'],
  );
});

test('importTargets skips guest memberships', () => {
  const guilds = [
    guild('a', 'https://lc.test'),
    guild('b', 'https://lc.test', { guestMode: true }),
    guild('c', 'https://lc.test', { role: 'guest' }),
    guild('d', 'https://lc.test', { role: 'admin' }),
  ];
  assert.deepEqual(
    importTargets(guilds, 'a').map((entry) => entry.membership.guildId),
    ['d'],
  );
  assert.deepEqual(importTargets(guilds, 'unknown'), []);
});

test('batches splits by 50', () => {
  const ids = Array.from({ length: 120 }, (_, index) => `id-${index}`);
  const parts = batches(ids);
  assert.deepEqual(parts.map((part) => part.length), [50, 50, 20]);
  assert.deepEqual(parts.flat(), ids);
  assert.deepEqual(batches([]), []);
});

test('importSummary sums the results', () => {
  const summary = importSummary(
    [{ status: 'added' }, { status: 'added' }, { status: 'already' }, { status: 'failed', error: 'media_ingestion_failed' }],
    'Les Thons',
  );
  assert.equal(summary.message, '2 médias ajoutés à Les Thons. · 1 déjà présent · 1 impossible');
  assert.equal(summary.isError, false);
  assert.equal(importSummary([{ status: 'added' }], 'Les Thons').message, '1 média ajouté à Les Thons.');
});

test('importSummary with nothing added is an error', () => {
  const summary = importSummary([{ status: 'not_found' }, { status: 'not_found' }, { status: 'failed' }], 'Les Thons');
  assert.equal(summary.message, 'Aucun média ajouté à Les Thons : 2 introuvables · 1 impossible');
  assert.equal(summary.isError, true);
  const already = importSummary([{ status: 'already' }, { status: 'already' }], 'Les Thons');
  assert.equal(already.message, 'Aucun média ajouté à Les Thons : 2 déjà présents');
  assert.equal(already.isError, false);
});

const keyState = (overrides = {}) => ({
  key: 'Escape',
  handled: false,
  capturing: false,
  dialogOpen: false,
  drawerOpen: false,
  selecting: false,
  ...overrides,
});

test('escapeAction closes the drawer first, then leaves the selection', () => {
  assert.equal(escapeAction(keyState({ drawerOpen: true, selecting: true })), 'closeDetail');
  assert.equal(escapeAction(keyState({ selecting: true })), 'leaveSelection');
  assert.equal(escapeAction(keyState()), null);
});

test('escapeAction ignores a key already handled by a context menu', () => {
  assert.equal(escapeAction(keyState({ handled: true, selecting: true })), null);
  assert.equal(escapeAction(keyState({ handled: true, drawerOpen: true })), null);
});

test('escapeAction leaves dialogs, shortcut capture and other keys alone', () => {
  assert.equal(escapeAction(keyState({ dialogOpen: true, selecting: true })), null);
  assert.equal(escapeAction(keyState({ capturing: true, drawerOpen: true })), null);
  assert.equal(escapeAction(keyState({ key: 'Enter', selecting: true })), null);
});

test('cardClickAction picks in selection mode and opens details otherwise', () => {
  assert.equal(cardClickAction({ selecting: true, shiftKey: false }), 'toggle');
  assert.equal(cardClickAction({ selecting: true, shiftKey: true }), 'range');
  assert.equal(cardClickAction({ selecting: false, shiftKey: true }), 'details');
});

test('menuTargetIds acts on the whole selection only from a picked card', () => {
  const order = ['a', 'b', 'c', 'd'];
  const picked = new Set(['c', 'a']);
  assert.deepEqual(menuTargetIds({ selecting: true, selection: picked, itemId: 'c', orderedIds: order }), ['a', 'c']);
  assert.deepEqual(menuTargetIds({ selecting: true, selection: picked, itemId: 'b', orderedIds: order }), ['b']);
  assert.deepEqual(menuTargetIds({ selecting: false, selection: picked, itemId: 'c', orderedIds: order }), ['c']);
});
