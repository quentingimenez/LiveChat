// Ce que la file doit afficher pendant une pause partagée : l’horloge et la
// barre se figent à la position que le serveur a gelée (pause média Windows ou
// bouton « Pause pour tous » de la board), puis repartent de cette position à
// la reprise. Le style de la barre doit couper l’animation CSS pendant la
// pause : sinon le compositeur continue d’avancer tout seul, même quand le
// texte ne bouge plus.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as queueClockModule from '../../web/app/core/queue-clock.js';

const { queueClock, queueProgressStyle } = queueClockModule;
const NOW = Date.parse('2026-11-20T21:00:00.000Z');
const startedAgo = (seconds) => new Date(NOW - seconds * 1000).toISOString();
const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const playing = (seconds = 2) => ({ startedAt: startedAgo(seconds), durationSec: 30 });

test('une pause fige l’horloge à la position gelée par le serveur', () => {
  const clock = queueClock(playing(2), NOW, { paused: true, remainingMs: 18_000 });
  assert.equal(clock.elapsed, 12);
  assert.equal(clock.ratio, 0.4);
  assert.equal(clock.text, '0:12 / 0:30');
});

test('l’horloge figée ne bouge plus, quel que soit le temps qui passe', () => {
  const pause = { paused: true, remainingMs: 18_000 };
  const first = queueClock(playing(2), NOW, pause);
  const later = queueClock(playing(2), NOW + 60_000, pause);
  assert.deepEqual(later, first);
});

test('le temps restant est borné à la durée du média', () => {
  const atStart = queueClock(playing(2), NOW, { paused: true, remainingMs: 30_000 });
  assert.equal(atStart.elapsed, 0);
  assert.equal(atStart.ratio, 0);
  const atEnd = queueClock(playing(2), NOW, { paused: true, remainingMs: -5_000 });
  assert.equal(atEnd.elapsed, 30);
  assert.equal(atEnd.ratio, 1);
});

test('hors pause, l’horloge repart de l’heure courante', () => {
  const resumed = queueClock(playing(2), NOW, { paused: false, remainingMs: 18_000 });
  assert.equal(resumed.elapsed, 2);
  assert.equal(resumed.ratio, 2 / 30);
});

test('le style coupe l’animation CSS pendant la pause', () => {
  assert.equal(typeof queueProgressStyle, 'function');
  const clock = queueClock(playing(12), NOW);
  const pausedStyle = queueProgressStyle(clock, true);
  assert.match(pausedStyle, /animation-name:none/);
  assert.match(pausedStyle, /transform:scaleX\(0\.4\)/);
  assert.doesNotMatch(pausedStyle, /--queue-progress-duration/);
});

test('le style garde l’animation de compositeur hors pause', () => {
  const clock = queueClock(playing(12), NOW);
  const style = queueProgressStyle(clock, false);
  assert.match(style, /--queue-progress-duration:30s/);
  assert.match(style, /--queue-progress-delay:-12s/);
  assert.match(style, /transform:scaleX\(0\.4\)/);
  assert.doesNotMatch(style, /animation-name:none/);
});

test('la file branche la pause sur l’horloge et le style', () => {
  const queue = read('views/queue.js');
  assert.match(queue, /pausedRemainingMs/);
  assert.match(queue, /queueProgressStyle\(clock, pause\.paused\)/);
  // A resume shifts `startedAt`: the read must supersede one already in flight,
  // otherwise the stale position could paint after the resume.
  assert.match(queue, /loadQueue\(\{ fresh: true \}\)/);
  assert.match(read('data.js'), /options\.fresh === true/);
  const tick = queue.match(/export function tickQueueClock\([\s\S]*?\r?\n\}/);
  assert.ok(tick, 'tickQueueClock est exporté');
  assert.match(tick[0], /animationName = 'none'/);
});
