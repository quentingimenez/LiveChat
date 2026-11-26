import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  batches,
  cardClickAction,
  cardPointerAction,
  copiedLinksSummary,
  deletableIds,
  deleteDialogText,
  deleteSummary,
  escapeAction,
  favoritePlan,
  importSummary,
  importTargets,
  menuTargetIds,
  originLinks,
  playSummary,
  pruneSelection,
  selectAllAction,
  selectRange,
  sourceLink,
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
  detailOpen: false,
  selecting: false,
  ...overrides,
});

test('escapeAction closes the detail window first, then leaves the selection', () => {
  assert.equal(escapeAction(keyState({ detailOpen: true, selecting: true })), 'closeDetail');
  assert.equal(escapeAction(keyState({ selecting: true })), 'leaveSelection');
  assert.equal(escapeAction(keyState()), null);
});

test('escapeAction ignores a key already handled by a context menu', () => {
  assert.equal(escapeAction(keyState({ handled: true, selecting: true })), null);
  assert.equal(escapeAction(keyState({ handled: true, detailOpen: true })), null);
});

test('escapeAction leaves dialogs, shortcut capture and other keys alone', () => {
  assert.equal(escapeAction(keyState({ dialogOpen: true, selecting: true })), null);
  assert.equal(escapeAction(keyState({ capturing: true, detailOpen: true })), null);
  assert.equal(escapeAction(keyState({ key: 'Enter', selecting: true })), null);
});

test('cardClickAction picks in selection mode and opens details otherwise', () => {
  assert.equal(cardClickAction({ selecting: true, shiftKey: false }), 'toggle');
  assert.equal(cardClickAction({ selecting: true, shiftKey: true }), 'range');
  assert.equal(cardClickAction({ selecting: false, shiftKey: true }), 'details');
  assert.equal(cardClickAction({ selecting: false, shiftKey: false }), 'details');
});

test('cardClickAction picks with Ctrl/Cmd even outside the selection mode', () => {
  assert.equal(cardClickAction({ selecting: false, shiftKey: false, ctrlKey: true }), 'pick');
  assert.equal(cardClickAction({ selecting: false, shiftKey: false, metaKey: true }), 'pick');
  assert.equal(cardClickAction({ selecting: true, shiftKey: false, ctrlKey: true }), 'toggle');
  assert.equal(cardClickAction({ selecting: true, shiftKey: false, metaKey: true }), 'toggle');
});

const pointerState = (overrides = {}) => ({
  selecting: false,
  clickCount: 1,
  shiftKey: false,
  ctrlKey: false,
  metaKey: false,
  onControl: false,
  ...overrides,
});

test('cardPointerAction treats missing modifier and control flags as off', () => {
  // The card handlers pass only what the event has: absent flags must not
  // read as a held Ctrl/Meta key or a click on a control.
  assert.equal(cardPointerAction({ selecting: false, clickCount: 2, shiftKey: false }), 'details');
  assert.equal(cardPointerAction({ selecting: false, clickCount: 0 }), 'details');
});

test('cardPointerAction ignores clicks on a card control', () => {
  assert.equal(cardPointerAction(pointerState({ onControl: true })), 'none');
  assert.equal(cardPointerAction(pointerState({ onControl: true, clickCount: 2 })), 'none');
  assert.equal(cardPointerAction(pointerState({ onControl: true, clickCount: 0 })), 'none');
  assert.equal(cardPointerAction(pointerState({ onControl: true, selecting: true })), 'none');
  assert.equal(cardPointerAction(pointerState({ onControl: true, ctrlKey: true })), 'none');
});

test('cardPointerAction opens the details on a double click or a keyboard click', () => {
  // A single mouse click does nothing on a card: playing left it for the Play
  // button and the preview.
  assert.equal(cardPointerAction(pointerState()), 'none');
  assert.equal(cardPointerAction(pointerState({ clickCount: 2 })), 'details');
  // Enter or Space on the thumbnail is a click without a count.
  assert.equal(cardPointerAction(pointerState({ clickCount: 0 })), 'details');
  // A triple click is not a new double click.
  assert.equal(cardPointerAction(pointerState({ clickCount: 3 })), 'none');
  assert.equal(cardPointerAction(pointerState({ shiftKey: true })), 'none');
  assert.equal(cardPointerAction(pointerState({ shiftKey: true, clickCount: 2 })), 'details');
});

test('cardPointerAction opens on a single click when the hover preview is off', () => {
  // The setting is off: the card no longer previews under the pointer, so a
  // single click opens the details at once and a double click plays the media.
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false })), 'details');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, clickCount: 0 })), 'details');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, clickCount: 2 })), 'play');
  // A triple click is neither a new single click nor a new double click.
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, clickCount: 3 })), 'none');
});

test('cardPointerAction opens the details at once and plays on the second click', () => {
  // The window is opened by the first click, so the second one lands on the
  // window: the board catches it there (see board-card-click.test.mjs). No
  // click waits for the double-click window.
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false })), 'details');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, shiftKey: true })), 'details');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, clickCount: 2 })), 'play');
});

test('cardPointerAction keeps a single click idle while the hover preview is on', () => {
  assert.equal(cardPointerAction(pointerState({ hoverPreview: true })), 'none');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: true, clickCount: 2 })), 'details');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: true, clickCount: 0 })), 'details');
});

test('the hover preview setting does not change the selection gestures', () => {
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, selecting: true })), 'toggle');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, selecting: true, shiftKey: true })), 'range');
  // The second click of a double click still does not pick a second time.
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, selecting: true, clickCount: 2 })), 'none');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, ctrlKey: true })), 'pick');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, metaKey: true })), 'pick');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, ctrlKey: true, clickCount: 2 })), 'none');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, onControl: true })), 'none');
  assert.equal(cardPointerAction(pointerState({ hoverPreview: false, onControl: true, clickCount: 2 })), 'none');
});

test('cardPointerAction keeps the selection gestures in the selection mode', () => {
  assert.equal(cardPointerAction(pointerState({ selecting: true })), 'toggle');
  assert.equal(cardPointerAction(pointerState({ selecting: true, shiftKey: true })), 'range');
  assert.equal(cardPointerAction(pointerState({ selecting: true, ctrlKey: true })), 'toggle');
  assert.equal(cardPointerAction(pointerState({ selecting: true, clickCount: 0 })), 'toggle');
  // The second click of a double click does not toggle a second time.
  assert.equal(cardPointerAction(pointerState({ selecting: true, clickCount: 2 })), 'none');
  assert.equal(cardPointerAction(pointerState({ selecting: true, shiftKey: true, clickCount: 2 })), 'none');
});

test('cardPointerAction picks with Ctrl/Cmd outside the selection mode', () => {
  assert.equal(cardPointerAction(pointerState({ ctrlKey: true })), 'pick');
  assert.equal(cardPointerAction(pointerState({ metaKey: true })), 'pick');
  assert.equal(cardPointerAction(pointerState({ ctrlKey: true, clickCount: 2 })), 'none');
  assert.equal(cardPointerAction(pointerState({ metaKey: true, clickCount: 2 })), 'none');
});

test('menuTargetIds acts on the whole selection only from a picked card', () => {
  const order = ['a', 'b', 'c', 'd'];
  const picked = new Set(['c', 'a']);
  assert.deepEqual(menuTargetIds({ selecting: true, selection: picked, itemId: 'c', orderedIds: order }), ['a', 'c']);
  assert.deepEqual(menuTargetIds({ selecting: true, selection: picked, itemId: 'b', orderedIds: order }), ['b']);
  assert.deepEqual(menuTargetIds({ selecting: false, selection: picked, itemId: 'c', orderedIds: order }), ['c']);
});

// --- bulk actions of the selection bar ---

// A board item, as the selection bar sees it.
const item = (id, { favorite = false, canEdit = false, sourceUrl = null } = {}) => ({
  id,
  favorite,
  canEdit,
  media: { sourceUrl },
});

test('favoritePlan removes only when every picked media is a favorite', () => {
  const plan = favoritePlan([item('a', { favorite: true }), item('b', { favorite: true })]);
  assert.equal(plan.label, 'Retirer des favoris');
  assert.equal(plan.favorite, false);
  assert.deepEqual(plan.ids, ['a', 'b']);
});

test('favoritePlan adds the favorites missing from the selection', () => {
  const plan = favoritePlan([item('a', { favorite: true }), item('b'), item('c')]);
  assert.equal(plan.label, 'Ajouter aux favoris');
  assert.equal(plan.favorite, true);
  assert.deepEqual(plan.ids, ['b', 'c']);
  assert.deepEqual(favoritePlan([]).ids, [], 'an empty selection has nothing to add');
});

test('deletableIds keeps the media the member may edit', () => {
  const items = [item('a', { canEdit: true }), item('b'), item('c', { canEdit: true })];
  assert.deepEqual(deletableIds(items), ['a', 'c']);
  assert.deepEqual(deletableIds([item('b')]), []);
});

test('sourceLink keeps the whole http(s) address, and nothing else', () => {
  const link = 'https://www.youtube.com/watch?v=abc&t=12';
  assert.equal(sourceLink(item('a', { sourceUrl: link })), link);
  assert.equal(sourceLink(item('b', { sourceUrl: 'HTTP://example.test/B' })), 'HTTP://example.test/B');
  // An uploaded file, a local file and a malformed source have no page.
  assert.equal(sourceLink(item('c', { sourceUrl: 'upload:clip.mp4' })), null);
  assert.equal(sourceLink(item('d', { sourceUrl: 'file:///tmp/d' })), null);
  assert.equal(sourceLink(item('e', { sourceUrl: 'pas une url' })), null);
  assert.equal(sourceLink(item('f')), null);
  assert.equal(sourceLink({ id: 'z' }), null, 'an item without media is linkless');
  assert.equal(sourceLink(null), null);
});

test('originLinks keeps the http(s) links, one per media, in board order', () => {
  const items = [
    item('a', { sourceUrl: 'https://example.test/a' }),
    item('b'),
    item('c', { sourceUrl: 'http://example.test/c' }),
    item('d', { sourceUrl: 'file:///tmp/d' }),
    item('e', { sourceUrl: 'https://example.test/e' }),
  ];
  assert.deepEqual(originLinks(items), ['https://example.test/a', 'http://example.test/c', 'https://example.test/e']);
  assert.deepEqual(originLinks([{ id: 'z' }]), [], 'a media without a source counts as linkless');
});

test('playSummary counts the media sent to the overlays', () => {
  assert.deepEqual(playSummary(3, 0), { message: '3 médias envoyés aux overlays.', isError: false });
  assert.deepEqual(playSummary(1, 0), { message: '1 média envoyé aux overlays.', isError: false });
});

test('playSummary reports the failures', () => {
  assert.deepEqual(playSummary(2, 1), { message: '2 médias envoyés aux overlays. · 1 échec', isError: true });
  assert.deepEqual(playSummary(0, 3), { message: 'Aucun média envoyé aux overlays : 3 échecs', isError: true });
});

test('deleteDialogText lists the count and the media without the right', () => {
  assert.deepEqual(deleteDialogText(4, 0), {
    title: 'Supprimer 4 médias ?',
    text: 'Ils seront retirés du board de la guilde.',
  });
  assert.deepEqual(deleteDialogText(4, 2), {
    title: 'Supprimer 4 médias ?',
    text: 'Ils seront retirés du board de la guilde. 2 médias ne peuvent pas être supprimés (droits).',
  });
  assert.deepEqual(deleteDialogText(1, 3), {
    title: 'Supprimer ce média ?',
    text: 'Ce média sera retiré du board de la guilde. 3 médias ne peuvent pas être supprimés (droits).',
  });
});

test('deleteSummary counts the deletions and the failures', () => {
  assert.deepEqual(deleteSummary(4, 0), { message: '4 médias supprimés.', isError: false });
  assert.deepEqual(deleteSummary(1, 0), { message: '1 média supprimé.', isError: false });
  assert.deepEqual(deleteSummary(3, 2), { message: '3 médias supprimés. · 2 échecs', isError: true });
  assert.deepEqual(deleteSummary(0, 2), { message: 'Aucun média supprimé : 2 échecs', isError: true });
});

test('copiedLinksSummary counts the copied links', () => {
  assert.equal(copiedLinksSummary(2), '2 liens copiés.');
  assert.equal(copiedLinksSummary(1), '1 lien copié.');
});

const selectAllState = (overrides = {}) => ({
  key: 'a',
  ctrl: true,
  meta: false,
  handled: false,
  capturing: false,
  dialogOpen: false,
  editableTarget: false,
  canSelectAll: true,
  ...overrides,
});

test('selectAllAction picks every visible card on Ctrl+A on the board', () => {
  assert.equal(selectAllAction(selectAllState()), 'selectAll');
  assert.equal(selectAllAction(selectAllState({ key: 'A' })), 'selectAll');
  assert.equal(selectAllAction(selectAllState({ ctrl: false, meta: true })), 'selectAll');
});

test('selectAllAction leaves other keys, pages and modifiers alone', () => {
  assert.equal(selectAllAction(selectAllState({ canSelectAll: false })), null);
  assert.equal(selectAllAction(selectAllState({ key: 'z' })), null);
  assert.equal(selectAllAction(selectAllState({ ctrl: false, meta: false })), null);
});

test('selectAllAction keeps handled keys, dialogs, captures and fields', () => {
  assert.equal(selectAllAction(selectAllState({ handled: true })), null);
  assert.equal(selectAllAction(selectAllState({ dialogOpen: true })), null);
  assert.equal(selectAllAction(selectAllState({ capturing: true })), null);
  assert.equal(selectAllAction(selectAllState({ editableTarget: true })), null);
});
