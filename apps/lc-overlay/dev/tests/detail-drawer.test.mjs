// Unit tests of the board's detail drawer: what a click, a hover and a leave
// show.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  cardClicked,
  cardHovered,
  CLOSED_VIEW,
  drawerUsed,
  isPeeking,
  pointerLeft,
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
