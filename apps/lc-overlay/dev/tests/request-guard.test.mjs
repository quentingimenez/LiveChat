// Ce que ce fichier protège : une lecture lente ne repeint jamais par-dessus une
// lecture plus récente, deux lectures simultanées de la même ressource ne
// partent qu’une fois, et une lecture peut expirer sans bloquer l’interface.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  dedupe,
  isLatest,
  pendingRequestCount,
  requestToken,
  withDeadline,
} from '../../web/app/core/query.js';

test('seul le dernier jeton d’une clé est courant', () => {
  const first = requestToken('items:g1');
  assert.equal(isLatest(first), true);
  const second = requestToken('items:g1');
  assert.equal(isLatest(first), false);
  assert.equal(isLatest(second), true);
});

test('les jetons de clés différentes ne se masquent pas', () => {
  const items = requestToken('items:g1');
  const queue = requestToken('queue:g1');
  assert.equal(isLatest(items), true);
  assert.equal(isLatest(queue), true);
});

test('un jeton absent n’est jamais courant', () => {
  assert.equal(isLatest(null), false);
  assert.equal(isLatest(undefined), false);
});

test('dedupe partage la même promesse tant que la lecture est en vol', async () => {
  let calls = 0;
  let release;
  const work = () => {
    calls += 1;
    return new Promise((resolve) => {
      release = () => resolve('ok');
    });
  };
  const first = dedupe('items:g1', work);
  const second = dedupe('items:g1', work);
  assert.equal(first, second);
  assert.equal(calls, 1);
  assert.equal(pendingRequestCount(), 1);
  release();
  assert.equal(await first, 'ok');
  assert.equal(pendingRequestCount(), 0);
});

test('dedupe relance la lecture une fois la précédente terminée', async () => {
  let calls = 0;
  const work = async () => {
    calls += 1;
    return calls;
  };
  assert.equal(await dedupe('queue:g1', work), 1);
  assert.equal(await dedupe('queue:g1', work), 2);
});

test('un échec libère la clé pour la tentative suivante', async () => {
  let calls = 0;
  const work = async () => {
    calls += 1;
    if (calls === 1) {
      throw new Error('boom');
    }
    return 'ok';
  };
  await assert.rejects(dedupe('info:g1', work), /boom/);
  assert.equal(pendingRequestCount(), 0);
  assert.equal(await dedupe('info:g1', work), 'ok');
});

test('withDeadline rend la valeur quand la lecture est assez rapide', async () => {
  assert.equal(await withDeadline(Promise.resolve('ok'), 50), 'ok');
});

test('withDeadline rejette avec le code timeout quand la lecture traîne', async () => {
  let released;
  const slow = new Promise((resolve) => {
    released = resolve;
  });
  await assert.rejects(withDeadline(slow, 5, 'timeout'), (error) => {
    assert.equal(error.code, 'timeout');
    return true;
  });
  // La promesse d'origine peut encore se terminer sans effet.
  released('tard');
});
