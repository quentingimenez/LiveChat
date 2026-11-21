// Loading, empty and error are three different states. Before this change the
// board said « Chargement du board… » in a plain paragraph and an empty queue
// looked exactly like a queue that had not been read yet: a slow server made
// the board look empty and the queue look idle.
//
// The views need a DOM and the harness has none: the states are pinned on the
// source that branches on them.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const board = read('views/board.js');
const queue = read('views/queue.js');
const admin = read('views/admin.js');
const api = read('core/api.js');
const css = read('styles/loading.css');

function section(source, from, to) {
  const start = source.indexOf(from);
  assert.ok(start >= 0, `${from} not found`);
  const end = source.indexOf(to, start);
  assert.ok(end > start, `${to} not found after ${from}`);
  return source.slice(start, end);
}

test('the board paints a skeleton before its first answer', () => {
  assert.match(board, /!state\.itemsLoaded && !state\.itemsError/);
  assert.match(board, /class: 'grid board-skeleton'/);
  assert.match(board, /Chargement du board…/);
  // The "empty board" branch comes after the skeleton branch: an unread board
  // can never be shown as an empty one.
  const skeleton = board.indexOf('board-skeleton');
  const empty = board.indexOf('Le board est vide');
  assert.ok(skeleton >= 0 && empty > skeleton, 'skeleton before the empty state');
});

test('a board read failure is an error with a retry, not an empty board', () => {
  assert.match(board, /state\.itemsError && state\.items\.length === 0/);
  assert.match(board, /Board indisponible/);
  assert.match(board, /onClick: retryItems/);
  assert.match(board, /state\.itemsLoaded = false;\s+state\.itemsError = null;/);
});

test('a failed refresh keeps the last board and marks it stale', () => {
  assert.match(board, /state\.itemsError && \(state\.items\.length > 0 \|\| uploads\.length > 0\)/);
  assert.match(board, /Impossible d’actualiser le board\./);
  assert.match(board, /class: 'board-banner is-stale'/);
});

test('the queue shows a skeleton until it is read', () => {
  assert.match(queue, /!state\.queueLoaded && !state\.queueError/);
  assert.match(queue, /queueSkeleton\(\)/);
  assert.match(queue, /Chargement de la file…/);
  const skeleton = queue.indexOf('queueSkeleton()');
  const empty = queue.indexOf('La file est vide');
  assert.ok(skeleton >= 0 && empty > skeleton, 'skeleton before the empty state');
});

test('a queue read failure is an error with a retry', () => {
  assert.match(queue, /state\.queueError && !playing && pending\.length === 0/);
  assert.match(queue, /File indisponible/);
  assert.match(queue, /onClick: retryQueue/);
  assert.match(queue, /state\.queueLoaded = false;\s+state\.queueError = null;/);
});

test('the administration lists show rows-shaped skeletons', () => {
  assert.match(admin, /list-state is-loading/);
  assert.match(admin, /Chargement…/);
  assert.match(admin, /is-skeleton/);
  assert.match(admin, /aria-busy/);
});

test('every action started by a button gets an in-flight mark', () => {
  assert.match(api, /document\.activeElement/);
  assert.match(api, /control\.setAttribute\('data-pending', 'true'\)/);
  assert.match(api, /control\.disabled = true/);
  assert.match(api, /control\.removeAttribute\('data-pending'\)/);
  assert.match(api, /control\.disabled = wasDisabled/);
});

test('the shared loading styles cover the three signals', () => {
  assert.match(css, /\.activity-bar/);
  assert.match(css, /\.skeleton-block/);
  assert.match(css, /button\[data-pending='true'\]/);
  assert.match(css, /prefers-reduced-motion: reduce/);
  assert.match(css, /\.sr-only/);
});
