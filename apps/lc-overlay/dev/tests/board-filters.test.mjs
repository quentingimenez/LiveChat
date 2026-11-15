import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  activeFilterCount,
  authorsOf,
  DEFAULT_FILTERS,
  droppedLink,
  durationBucket,
  mediaSource,
  selectItems,
} from '../../web/app/core/board-filters.js';

const items = [
  { id: 'a', title: 'Zèbre', createdByName: 'Léa', createdAt: '2026-01-03T00:00:00Z', favorite: true, media: { kind: 'video', durationSec: 5, sourceUrl: 'https://www.youtube.com/watch?v=1' } },
  { id: 'b', title: 'abeille', createdByName: 'Marc', createdAt: '2026-01-01T00:00:00Z', favorite: false, media: { kind: 'audio', durationSec: 42, sourceUrl: 'upload:son.mp3' } },
  { id: 'c', title: 'Chat', createdByName: 'Léa', createdAt: '2026-01-02T00:00:00Z', favorite: false, media: { kind: 'image', durationSec: null, sourceUrl: 'https://x.com/a/status/1' } },
  { id: 'd', title: 'Danse', createdByName: null, createdAt: '2026-01-04T00:00:00Z', favorite: true, media: { kind: 'video', durationSec: 75, sourceUrl: 'https://vm.tiktok.com/abc' } },
];

const ids = (list) => list.map((item) => item.id);

test('mediaSource recognizes the platforms', () => {
  assert.equal(mediaSource('https://youtu.be/x'), 'youtube');
  assert.equal(mediaSource('https://m.youtube.com/watch?v=x'), 'youtube');
  assert.equal(mediaSource('https://vm.tiktok.com/x'), 'tiktok');
  assert.equal(mediaSource('https://twitter.com/a/status/1'), 'x');
  assert.equal(mediaSource('upload:clip.mp4'), 'file');
  assert.equal(mediaSource('https://cdn.example.com/a.gif'), 'direct');
  assert.equal(mediaSource(null), 'file');
});

test('durationBucket splits at 10 s and 1 min', () => {
  assert.equal(durationBucket(9.9), 'short');
  assert.equal(durationBucket(10), 'medium');
  assert.equal(durationBucket(60), 'medium');
  assert.equal(durationBucket(61), 'long');
  assert.equal(durationBucket(null), null);
});

test('default selection is newest first', () => {
  assert.deepEqual(ids(selectItems(items, {})), ['d', 'a', 'c', 'b']);
});

test('filters combine', () => {
  const favorites = { ...DEFAULT_FILTERS, favorites: true };
  assert.deepEqual(ids(selectItems(items, { filters: favorites })), ['d', 'a']);
  const lea = { ...DEFAULT_FILTERS, author: 'Léa' };
  assert.deepEqual(ids(selectItems(items, { filters: lea })), ['a', 'c']);
  const file = { ...DEFAULT_FILTERS, source: 'file' };
  assert.deepEqual(ids(selectItems(items, { filters: file })), ['b']);
  const long = { ...DEFAULT_FILTERS, duration: 'long' };
  assert.deepEqual(ids(selectItems(items, { filters: long })), ['d']);
  const shortcut = { ...DEFAULT_FILTERS, shortcut: true };
  assert.deepEqual(ids(selectItems(items, { filters: shortcut, shortcutItemIds: new Set(['c']) })), ['c']);
  assert.deepEqual(ids(selectItems(items, { kind: 'video', filters: favorites, search: 'dan' })), ['d']);
});

test('sorts by title, age and duration', () => {
  assert.deepEqual(ids(selectItems(items, { filters: { ...DEFAULT_FILTERS, sort: 'title' } })), ['b', 'c', 'd', 'a']);
  assert.deepEqual(ids(selectItems(items, { filters: { ...DEFAULT_FILTERS, sort: 'oldest' } })), ['b', 'c', 'a', 'd']);
  // Images (no duration) come last.
  assert.deepEqual(ids(selectItems(items, { filters: { ...DEFAULT_FILTERS, sort: 'duration' } })), ['a', 'b', 'd', 'c']);
});

test('authors and active filter count', () => {
  assert.deepEqual(authorsOf(items), ['Léa', 'Marc']);
  assert.equal(activeFilterCount(DEFAULT_FILTERS), 0);
  assert.equal(activeFilterCount({ ...DEFAULT_FILTERS, favorites: true, source: 'x', sort: 'title' }), 2);
});

test('droppedLink takes the first web URL', () => {
  assert.equal(droppedLink('https://youtu.be/abc'), 'https://youtu.be/abc');
  assert.equal(droppedLink('# comment\r\nhttps://x.com/a/status/1\r\n'), 'https://x.com/a/status/1');
  assert.equal(droppedLink('file:///C:/a.mp4'), null);
  assert.equal(droppedLink('du texte'), null);
  assert.equal(droppedLink(undefined), null);
});
