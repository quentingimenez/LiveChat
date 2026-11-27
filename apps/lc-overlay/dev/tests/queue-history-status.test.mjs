// Statut d'une ligne d'historique de file (Node's built-in test runner).
//
// Ce que ce fichier protège : un média retiré de la file par un opérateur n'est
// pas un échec de lecture. Le serveur le passe `FAILED` (seule transition
// `PENDING → terminal` admise) et le marque `removedAt`/`removedBy` ; le rendu
// doit le dire « Retiré », avec un glyphe neutre et le nom de l'auteur du
// retrait quand il est connu. Un `FAILED` sans marqueur reste un échec, et un
// job terminé reste « Terminé ».

import assert from 'node:assert/strict';
import test from 'node:test';

import { historyStatus } from '../../web/app/core/history-status.js';

test('un retrait de file est affiché « Retiré », pas « Échec »', () => {
  const status = historyStatus({
    status: 'failed',
    removedAt: '2024-11-24T19:16:41.000Z',
    removedBy: 'Alice',
  });
  assert.equal(status.kind, 'removed');
  assert.equal(status.label, 'Retiré');
  assert.equal(status.title, 'Retiré de la file par Alice');
  assert.equal(status.className, 'is-muted');
  assert.equal(status.icon, 'remove');
});

test('un retrait sans auteur connu garde un titre neutre', () => {
  const status = historyStatus({ status: 'failed', removedAt: '2024-11-24T19:16:41.000Z' });
  assert.equal(status.kind, 'removed');
  assert.equal(status.title, 'Retiré de la file');
  assert.equal(status.className, 'is-muted');
});

test('le marqueur prime sur le statut : un retrait reste un retrait', () => {
  // La base écrit `FAILED` pour un retrait : c'est le marqueur qui tranche.
  const removed = historyStatus({ status: 'failed', removedAt: '2024-11-24T19:51:22.000Z' });
  const failed = historyStatus({ status: 'failed' });
  assert.notEqual(removed.kind, failed.kind);
  assert.equal(failed.label, 'Échec');
  assert.equal(failed.className, 'is-err');
  assert.equal(failed.icon, 'close');
  // Et le titre d'un vrai échec ne parle jamais d'un retrait.
  assert.equal(failed.title, 'Échec');
});

test('un échec sans marqueur reste « Échec »', () => {
  const status = historyStatus({ status: 'failed', removedAt: null, removedBy: null });
  assert.equal(status.kind, 'failed');
  assert.equal(status.className, 'is-err');
  assert.equal(status.icon, 'close');
});

test('un job terminé reste « Terminé »', () => {
  const status = historyStatus({ status: 'done' });
  assert.equal(status.kind, 'done');
  assert.equal(status.label, 'Terminé');
  assert.equal(status.className, 'is-ok');
  assert.equal(status.icon, 'check');
});

test('une ligne sans job ne casse pas le rendu', () => {
  assert.equal(historyStatus(null).kind, 'done');
  assert.equal(historyStatus(undefined).className, 'is-ok');
});
