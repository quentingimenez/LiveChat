// Contrôles explicites de la file (Node's built-in test runner).
//
// Ce que ce fichier protège : à côté de la poignée de glisser-déposer, chaque
// média en attente garde deux boutons Monter/Descendre qui envoient l'ordre
// complet au serveur, avec les bornes de la file. Les vues demandent un DOM :
// les assertions lisent la source comme les autres tests de la file.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const queue = readFileSync(new URL('../../web/app/views/queue.js', import.meta.url), 'utf8');

test('la file en attente offre Monter et Descendre', () => {
  assert.match(queue, /title: 'Monter'/);
  assert.match(queue, /icon\('up'\)/);
  assert.match(queue, /title: 'Descendre'/);
  assert.match(queue, /icon\('down'\)/);
});

test('les contrôles déplacent le job et envoient l’ordre complet', () => {
  assert.match(queue, /onClick: \(event\) => moveJob\(job, -1, event\.currentTarget\)/);
  assert.match(queue, /onClick: \(event\) => moveJob\(job, 1, event\.currentTarget\)/);
});

test('les bornes désactivent Monter en tête et Descendre en queue', () => {
  assert.match(queue, /disabled: index === 0/);
  assert.match(queue, /disabled: index === pending\.length - 1/);
});
