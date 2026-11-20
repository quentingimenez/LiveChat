// What the queue clock must show, and the guarantee that the one-second tick
// updates it in place instead of rebuilding the queue view (a rebuild closed
// the open history and dropped keyboard focus).

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { queueClock } from '../../web/app/core/queue-clock.js';

const NOW = Date.parse('2026-11-20T21:00:00.000Z');
const startedAgo = (seconds) => new Date(NOW - seconds * 1000).toISOString();

test('l’horloge affiche le temps écoulé et la durée totale', () => {
  const clock = queueClock({ startedAt: startedAgo(10), durationSec: 30 }, NOW);
  assert.equal(clock.text, '0:10 / 0:30');
  assert.ok(Math.abs(clock.ratio - 1 / 3) < 0.001);
});

test('l’horloge s’arrête à la durée du média', () => {
  const clock = queueClock({ startedAt: startedAgo(90), durationSec: 30 }, NOW);
  assert.equal(clock.text, '0:30 / 0:30');
  assert.equal(clock.ratio, 1);
});

test('sans début connu, l’horloge reste à zéro', () => {
  const clock = queueClock({ durationSec: 30 }, NOW);
  assert.equal(clock.text, '0:00 / 0:30');
  assert.equal(clock.ratio, 0);
});

test('une durée absente ou nulle ne fait pas diviser par zéro', () => {
  for (const durationSec of [0, null, undefined, Number.NaN, 'nope']) {
    const clock = queueClock({ startedAt: startedAgo(5), durationSec }, NOW);
    assert.equal(clock.text, '0:00 / 0:00');
    assert.equal(clock.ratio, 0);
  }
});

test('aucun média en lecture : horloge neutre', () => {
  assert.deepEqual(queueClock(null, NOW), { text: '0:00 / 0:00', ratio: 0 });
});

test('la seconde de battement met à jour le temps, pas toute la vue', () => {
  const main = readFileSync(new URL('../../web/app/main.js', import.meta.url), 'utf8');
  const block = main.match(/setInterval\(\(\) => \{[\s\S]*?tickQueueClock\(\);[\s\S]*?\}, 1000\);/);
  assert.ok(block, 'le battement d’une seconde appelle tickQueueClock');
  assert.doesNotMatch(block[0], /renderView\(\)/);
});

test('la file expose des cibles de mise à jour et garde l’historique ouvert', () => {
  const queue = readFileSync(new URL('../../web/app/views/queue.js', import.meta.url), 'utf8');
  assert.match(queue, /'data-queue-clock': ''/);
  assert.match(queue, /'data-queue-progress': ''/);
  assert.match(queue, /open: historyOpen/);
});
