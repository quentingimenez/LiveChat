// Ce que ce fichier protège : le cache par guilde sert un instantané immédiat
// au changement de guilde, sans jamais confondre une valeur fraîche, une valeur
// périmée et une absence de valeur.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CACHE_TTL,
  MAX_GUILDS,
  clearGuildCache,
  guildCacheSize,
  invalidateGuildCache,
  pruneGuildCache,
  readGuildCache,
  writeGuildCache,
} from '../../web/app/core/guild-cache.js';

test.beforeEach(() => {
  clearGuildCache();
});

test('une guilde sans écriture ne renvoie rien', () => {
  assert.equal(readGuildCache('g1', 'info', 1_000), null);
});

test('une valeur écrite est fraîche pendant son TTL', () => {
  writeGuildCache('g1', 'info', { name: 'Les Sardines' }, 1_000);
  const cached = readGuildCache('g1', 'info', 1_000 + CACHE_TTL.info);
  assert.deepEqual(cached.value, { name: 'Les Sardines' });
  assert.equal(cached.at, 1_000);
  assert.equal(cached.age, CACHE_TTL.info);
  assert.equal(cached.fresh, true);
});

test('une valeur plus vieille que son TTL reste lisible mais n’est plus fraîche', () => {
  writeGuildCache('g1', 'items', [{ id: 'a' }], 1_000);
  const cached = readGuildCache('g1', 'items', 1_000 + CACHE_TTL.items + 1);
  assert.deepEqual(cached.value, [{ id: 'a' }]);
  assert.equal(cached.fresh, false);
});

test('les clés ont des TTL indépendants', () => {
  writeGuildCache('g1', 'queue', { pending: [] }, 0);
  assert.equal(readGuildCache('g1', 'queue', CACHE_TTL.queue + 1).fresh, false);
  assert.equal(readGuildCache('g1', 'queue', CACHE_TTL.queue).fresh, true);
});

test('invalider une clé ne touche pas les autres', () => {
  writeGuildCache('g1', 'info', { name: 'A' }, 1_000);
  writeGuildCache('g1', 'items', [{ id: 'a' }], 1_000);
  invalidateGuildCache('g1', 'info');
  assert.equal(readGuildCache('g1', 'info', 1_000), null);
  assert.deepEqual(readGuildCache('g1', 'items', 1_000).value, [{ id: 'a' }]);
});

test('invalider sans clé oublie toute la guilde', () => {
  writeGuildCache('g1', 'info', { name: 'A' }, 1_000);
  writeGuildCache('g1', 'items', [], 1_000);
  invalidateGuildCache('g1');
  assert.equal(readGuildCache('g1', 'info', 1_000), null);
  assert.equal(guildCacheSize(), 0);
});

test('le cache garde au plus MAX_GUILDS guildes, la moins récemment lue part en premier', () => {
  for (let index = 0; index < MAX_GUILDS; index += 1) {
    writeGuildCache(`g${index}`, 'info', { index }, index * 10);
  }
  // Touch g0 so g1 becomes the least recently used entry.
  readGuildCache('g0', 'info', 1_000);
  writeGuildCache('extra', 'info', { index: 99 }, 1_010);
  assert.equal(guildCacheSize(), MAX_GUILDS);
  assert.equal(readGuildCache('g1', 'info', 1_020), null);
  assert.deepEqual(readGuildCache('g0', 'info', 1_020).value, { index: 0 });
  assert.deepEqual(readGuildCache('extra', 'info', 1_020).value, { index: 99 });
});

test('prune ne garde que les guildes encore connues', () => {
  writeGuildCache('g1', 'info', { name: 'A' });
  writeGuildCache('g2', 'info', { name: 'B' });
  pruneGuildCache(['g2']);
  assert.equal(readGuildCache('g1', 'info'), null);
  assert.deepEqual(readGuildCache('g2', 'info').value, { name: 'B' });
});

test('une clé inconnue n’est jamais écrite', () => {
  writeGuildCache('g1', 'inconnue', { value: true });
  assert.equal(readGuildCache('g1', 'inconnue'), null);
  assert.equal(guildCacheSize(), 0);
});
