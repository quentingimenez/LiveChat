// Unit tests of the board's detail drawer: what a click, a hover and a leave
// show.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  cardClicked,
  cardHovered,
  CLOSED_VIEW,
  drawerUsed,
  artworkOf,
  isPeeking,
  mediaPath,
  pointerLeft,
  previewKey,
  pruneView,
  sameView,
  shownId,
} from '../../web/app/core/detail-drawer.js';

const pinnedOn = (id) => ({ selectedId: id, pinned: true, peekId: null });

test('nothing pinned: the drawer follows the pointer and closes when it leaves', () => {
  const hovered = cardHovered(CLOSED_VIEW, 'a');
  assert.deepEqual(hovered, { selectedId: 'a', pinned: false, peekId: null });
  assert.equal(shownId(hovered), 'a');
  assert.equal(isPeeking(hovered), true);
  assert.deepEqual(cardHovered(hovered, 'b'), { selectedId: 'b', pinned: false, peekId: null });
  assert.deepEqual(pointerLeft(hovered), CLOSED_VIEW);
});

test('a click pins a media, a click on the pinned one closes the drawer', () => {
  const pinned = cardClicked(CLOSED_VIEW, 'a');
  assert.deepEqual(pinned, pinnedOn('a'));
  assert.equal(isPeeking(pinned), false);
  assert.deepEqual(cardClicked(pinned, 'a'), CLOSED_VIEW);
  assert.deepEqual(cardClicked(pinned, 'b'), pinnedOn('b'));
});

test('regression: hovering another card still shows its details once a media is pinned', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.equal(shownId(peek), 'b');
  assert.equal(peek.selectedId, 'a');
  assert.equal(peek.pinned, true);
  assert.equal(isPeeking(peek), true);
  assert.equal(shownId(cardHovered(peek, 'c')), 'c');
});

test('the drawer returns to the pinned media when the pointer leaves or comes back on its card', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(pointerLeft(peek), pinnedOn('a'));
  assert.deepEqual(cardHovered(peek, 'a'), pinnedOn('a'));
  // Leaving without a peek keeps the pinned media.
  assert.deepEqual(pointerLeft(pinnedOn('a')), pinnedOn('a'));
});

test('clicking while peeking: the peeked card gets pinned, the pinned one closes', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(cardClicked(peek, 'b'), pinnedOn('b'));
  assert.deepEqual(cardClicked(peek, 'a'), CLOSED_VIEW);
});

test('a double click always leaves the drawer pinned on its media', () => {
  // The browser sends click (detail 1), click (detail 2), then dblclick.
  const twice = (view, id) => cardClicked(cardClicked(view, id, 1), id, 2);
  assert.deepEqual(twice(CLOSED_VIEW, 'a'), pinnedOn('a'));
  // Regression: the two clicks used to toggle twice (pin then unpin).
  assert.deepEqual(twice(pinnedOn('a'), 'a'), pinnedOn('a'));
  assert.deepEqual(twice(pinnedOn('b'), 'a'), pinnedOn('a'));
  assert.deepEqual(twice(cardHovered(pinnedOn('b'), 'a'), 'a'), pinnedOn('a'));
  assert.deepEqual(twice(cardHovered(CLOSED_VIEW, 'a'), 'a'), pinnedOn('a'));
  // A third click of the same burst changes nothing.
  assert.deepEqual(cardClicked(twice(CLOSED_VIEW, 'a'), 'a', 3), pinnedOn('a'));
  // A keyboard activation (detail 0) or a call without a count toggles.
  assert.deepEqual(cardClicked(pinnedOn('a'), 'a', 0), CLOSED_VIEW);
  assert.deepEqual(cardClicked(pinnedOn('a'), 'a'), CLOSED_VIEW);
});

test('using the drawer pins what it shows', () => {
  assert.deepEqual(drawerUsed(cardHovered(pinnedOn('a'), 'b')), pinnedOn('b'));
  assert.deepEqual(drawerUsed(cardHovered(CLOSED_VIEW, 'a')), pinnedOn('a'));
  assert.deepEqual(drawerUsed(pinnedOn('a')), pinnedOn('a'));
  assert.deepEqual(drawerUsed(CLOSED_VIEW), CLOSED_VIEW);
});

test('pruneView drops a peek on a deleted media', () => {
  const peek = cardHovered(pinnedOn('a'), 'b');
  assert.deepEqual(pruneView(peek, (id) => id === 'a'), pinnedOn('a'));
  assert.equal(pruneView(peek, () => true), peek);
  // A peek left behind by a drawer that is no longer pinned is dropped.
  assert.deepEqual(pruneView({ selectedId: 'a', pinned: false, peekId: 'b' }, () => true), { selectedId: 'a', pinned: false, peekId: null });
});

test('sameView compares the three fields', () => {
  assert.equal(sameView(pinnedOn('a'), pinnedOn('a')), true);
  assert.equal(sameView(pinnedOn('a'), cardHovered(pinnedOn('a'), 'b')), false);
});

// --- preview of the shown media ---
const audio = (artwork) => ({
  id: 'a',
  mediaUrl: 'https://srv.test/media/a1?exp=100&sig=x',
  media: { kind: 'audio' },
  artwork,
});

test('artworkOf: only a picture with a URL counts', () => {
  assert.equal(artworkOf(audio(null)), null);
  assert.equal(artworkOf(audio(undefined)), null);
  assert.equal(artworkOf(audio({ url: '' })), null);
  assert.equal(artworkOf(audio({ url: null, width: 1, height: 1 })), null);
  assert.equal(artworkOf(audio({ url: 'https://srv.test/media/p1?sig=y' })).url, 'https://srv.test/media/p1?sig=y');
});

test('mediaPath drops the signature of a URL', () => {
  assert.equal(mediaPath('https://srv.test/media/a1?exp=100&sig=x'), 'https://srv.test/media/a1');
  assert.equal(mediaPath(''), null);
  assert.equal(mediaPath(null), null);
});

test('previewKey: a new signature keeps the preview', () => {
  const before = audio({ url: 'https://srv.test/media/p1?exp=100&sig=x' });
  const after = { ...before, mediaUrl: 'https://srv.test/media/a1?exp=900&sig=z', artwork: { url: 'https://srv.test/media/p1?exp=900&sig=z' } };
  assert.equal(previewKey(before), previewKey(after));
});

test('regression: removing, adding or replacing the picture of an audio rebuilds its preview', () => {
  const withPicture = audio({ url: 'https://srv.test/media/p1?sig=x' });
  const without = audio(null);
  const replaced = audio({ url: 'https://srv.test/media/p2?sig=x' });
  assert.notEqual(previewKey(withPicture), previewKey(without));
  assert.notEqual(previewKey(withPicture), previewKey(replaced));
});

test('previewKey: the picture only matters for an audio', () => {
  const video = { id: 'v', mediaUrl: 'https://srv.test/media/v1?sig=x', media: { kind: 'video' }, artwork: null };
  assert.equal(previewKey(video), previewKey({ ...video, artwork: { url: 'https://srv.test/media/p1' } }));
});
