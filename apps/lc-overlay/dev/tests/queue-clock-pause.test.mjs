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

const { queueClock, queueProgressStyle, resumedPlaying } = queueClockModule;
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

test('la reprise ancre la barre sur la position gelée sans attendre la relecture', () => {
  const before = { id: 'j0', title: 'Chat qui tombe', startedAt: startedAgo(2), durationSec: 30 };
  const resumed = resumedPlaying(before, 12, NOW);
  assert.equal(resumed.startedAt, new Date(NOW - 12_000).toISOString());
  // The frozen position is kept, and the original snapshot is not mutated.
  assert.equal(resumed.id, 'j0');
  assert.equal(resumed.title, 'Chat qui tombe');
  assert.equal(before.startedAt, startedAgo(2));
  const clock = queueClock(resumed, NOW);
  assert.equal(clock.elapsed, 12);
  assert.equal(clock.ratio, 0.4);
  assert.equal(clock.text, '0:12 / 0:30');
});

test('une position de reprise inconnue laisse la file intacte', () => {
  const before = playing(2);
  assert.equal(resumedPlaying(before, null, NOW), before);
  assert.equal(resumedPlaying(before, Number.NaN, NOW), before);
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
  // and it must not repaint the queue when the bar resumed locally (that would
  // reset the running animation); without a frozen position it has to repaint.
  assert.match(queue, /loadQueue\(\{[^}]*fresh: true[^}]*render: !resumed[^}]*\}\)/);
  assert.match(queue, /resumeReloadPending = !resumed/);
  assert.match(read('data.js'), /options\.fresh === true/);
  assert.match(read('data.js'), /options\.render !== false/);
  const tick = queue.match(/export function tickQueueClock\([\s\S]*?\r?\n\}/);
  assert.ok(tick, 'tickQueueClock est exporté');
  assert.match(tick[0], /animationName = 'none'/);
  assert.match(queue, /resumedPlaying\(playing, frozenElapsed, now\)/);
  assert.match(queue, /resumeQueueAnimation\(frozen\)/);
});
