// What the queue clock must show, and the guarantee that the one-second tick
// updates it in place instead of rebuilding the queue view (a rebuild closed
// the open history and dropped keyboard focus).

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { queueClock } from '../../web/app/core/queue-clock.js';

const NOW = Date.parse('2026-11-20T21:00:00.000Z');
const startedAgo = (seconds) => new Date(NOW - seconds * 1000).toISOString();
const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

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
  assert.deepEqual(queueClock(null, NOW), { text: '0:00 / 0:00', ratio: 0, elapsed: 0, duration: 0, anchored: false });
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

test('la position de la barre est exacte, sans arrondi au pourcentage', () => {
  const clock = queueClock({ startedAt: startedAgo(10), durationSec: 30 }, NOW);
  assert.equal(clock.elapsed, 10);
  assert.equal(clock.duration, 30);
  assert.equal(clock.anchored, true);
  assert.equal(clock.ratio, 1 / 3);
});

test('la barre avance en continu au lieu de sauter chaque seconde', () => {
  const queue = read('views/queue.js');
  // The fill advances over the real duration: a delay equal to the elapsed time
  // places it where playback already is, even after a re-render.
  assert.match(queue, /--queue-progress-duration/);
  assert.match(queue, /--queue-progress-delay/);
  assert.match(queue, /transform:scaleX\(/);
  // No integer percentage recomputed once a second: on a 30 s clip it moved by
  // 3 % or 4 % per tick, and beyond 100 s it only moved every two or three ticks.
  assert.doesNotMatch(queue, /Math\.round\(clock\.ratio \* 100\)/);
  assert.doesNotMatch(queue, /\.style\.width =/);

  const css = read('styles/queue.css');
  const span = css.match(/\.progress > span \{[\s\S]*?\r?\n\}/);
  assert.ok(span, 'la règle .progress > span existe');
  assert.match(span[0], /animation-name: queue-progress/);
  assert.match(span[0], /animation-timing-function: linear/);
  assert.match(span[0], /transform-origin: left/);
  assert.match(css, /@keyframes queue-progress \{[\s\S]*?scaleX\(0\)[\s\S]*?scaleX\(1\)[\s\S]*?\}/);
});

test('la barre reste animée quand les animations système sont réduites', () => {
  const css = read('styles/queue.css');
  // The fill reports progress: freezing it would hide how far playback is and
  // leave the one-second tick's jumps as the only movement. The animation is
  // therefore kept even under prefers-reduced-motion.
  assert.doesNotMatch(
    css,
    /@media \(prefers-reduced-motion: reduce\) \{[\s\S]*?\.progress > span[\s\S]*?\}/,
  );
});

test('le battement d’une seconde pose la position exacte de la barre', () => {
  const queue = read('views/queue.js');
  const tick = queue.match(/export function tickQueueClock\([\s\S]*?\r?\n\}/);
  assert.ok(tick, 'tickQueueClock est exporté');
  assert.match(tick[0], /bar\.style\.transform = `scaleX\(\$\{clock\.ratio\}\)`/);
  assert.doesNotMatch(tick[0], /width/);
});
