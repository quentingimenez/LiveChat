// Guards the pending-queue reordering helpers used by the overlay: a moderator
// drags a media onto another row (or moves it one step with the keyboard), and
// the overlay sends the whole list to the server. The helpers must never drop a
// job, duplicate one, or move past the ends.

import assert from 'node:assert/strict';
import test from 'node:test';

import { moveInQueue, moveToQueue, queueJobIds } from '../../web/app/core/queue-order.js';

const jobs = [{ id: 'a' }, { id: 'b' }, { id: 'c' }];
const ids = (list) => list.map((job) => job.id);

test('descend un média d’un cran', () => {
  assert.deepEqual(ids(moveInQueue(jobs, 'a', 1)), ['b', 'a', 'c']);
});

test('monte un média d’un cran', () => {
  assert.deepEqual(ids(moveInQueue(jobs, 'c', -1)), ['a', 'c', 'b']);
});

test('le premier ne peut pas monter, le dernier ne peut pas descendre', () => {
  assert.deepEqual(ids(moveInQueue(jobs, 'a', -1)), ['a', 'b', 'c']);
  assert.deepEqual(ids(moveInQueue(jobs, 'c', 1)), ['a', 'b', 'c']);
});

test('un identifiant inconnu ou un déplacement nul ne change rien', () => {
  assert.deepEqual(ids(moveInQueue(jobs, 'inconnu', 1)), ['a', 'b', 'c']);
  assert.deepEqual(ids(moveInQueue(jobs, 'b', 0)), ['a', 'b', 'c']);
});

test('la liste d’entrée n’est pas modifiée', () => {
  const before = ids(jobs);
  moveInQueue(jobs, 'a', 1);
  assert.deepEqual(ids(jobs), before);
});

test('la liste envoyée au serveur garde tous les médias dans l’ordre affiché', () => {
  const reordered = moveInQueue(jobs, 'a', 2);
  assert.deepEqual(queueJobIds(reordered), ['b', 'c', 'a']);
});

test('glisser un média sur une ligne le place à cette position', () => {
  assert.deepEqual(ids(moveToQueue(jobs, 'a', 'c')), ['b', 'c', 'a']);
  assert.deepEqual(ids(moveToQueue(jobs, 'c', 'a')), ['c', 'a', 'b']);
  assert.deepEqual(ids(moveToQueue(jobs, 'b', 'a')), ['b', 'a', 'c']);
});

test('déposer sur soi-même ou sur un inconnu ne change rien', () => {
  assert.deepEqual(ids(moveToQueue(jobs, 'b', 'b')), ['a', 'b', 'c']);
  assert.deepEqual(ids(moveToQueue(jobs, 'inconnu', 'a')), ['a', 'b', 'c']);
});

test('moveToQueue ne modifie pas la liste d’entrée', () => {
  const before = ids(jobs);
  moveToQueue(jobs, 'a', 'c');
  assert.deepEqual(ids(jobs), before);
});
