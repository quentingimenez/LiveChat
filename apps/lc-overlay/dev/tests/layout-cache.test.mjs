// Ce que ce fichier protège : le layout persistant ne garde que la forme des
// médias de la dernière visite (type + dimensions), survit à un redémarrage via
// localStorage, et reste borné (8 guildes, 60 médias) sans jamais casser le
// board si le stockage est absent, plein ou corrompu.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  LAYOUT_STORAGE_KEY,
  MAX_LAYOUT_GUILDS,
  MAX_LAYOUT_ITEMS,
  clearLayoutCache,
  layoutCacheSize,
  pruneLayoutCache,
  readLayoutCache,
  writeLayoutCache,
} from '../../web/app/core/layout-cache.js';

function fakeStorage(raw) {
  const values = new Map();
  if (raw !== undefined) {
    values.set(LAYOUT_STORAGE_KEY, raw);
  }
  return {
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    raw: () => values.get(LAYOUT_STORAGE_KEY),
  };
}

test('une guilde sans layout ne renvoie rien', () => {
  assert.equal(readLayoutCache('g1', fakeStorage()), null);
});

test('le layout ne conserve que le type et les dimensions', () => {
  const storage = fakeStorage();
  writeLayoutCache(
    'g1',
    [
      { id: 'a', title: 'Secret', url: 'https://cdn/x', kind: 'video', width: 1080, height: 1920 },
      { id: 'b', kind: 'audio', width: 0, height: 0 },
    ],
    storage,
    1_000,
  );
  const cached = readLayoutCache('g1', storage);
  assert.deepEqual(cached.items, [
    { kind: 'video', width: 1080, height: 1920 },
    { kind: 'audio', width: null, height: null },
  ]);
  assert.equal(cached.at, 1_000);
  const raw = storage.raw();
  assert.doesNotMatch(raw, /Secret|cdn\/x|"id"/);
});

test('le layout lit la forme réelle des items du board', () => {
  const storage = fakeStorage();
  writeLayoutCache(
    'g1',
    [
      {
        id: 'nested',
        title: 'Nested',
        media: { kind: 'video', width: 1920, height: 1080 },
      },
      {
        id: 'artwork',
        media: { kind: 'audio', width: 0, height: 0 },
        artwork: { width: 480, height: 480 },
      },
    ],
    storage,
  );
  assert.deepEqual(readLayoutCache('g1', storage).items, [
    { kind: 'video', width: 1920, height: 1080 },
    { kind: 'image', width: 480, height: 480 },
  ]);
});

test('le layout est borné au nombre de médias persistés', () => {
  const storage = fakeStorage();
  const items = Array.from({ length: MAX_LAYOUT_ITEMS + 20 }, (_, index) => ({
    kind: 'image',
    width: 100 + index,
    height: 100,
  }));
  writeLayoutCache('g1', items, storage);
  assert.equal(readLayoutCache('g1', storage).items.length, MAX_LAYOUT_ITEMS);
});

test('les guildes les plus récentes remplacent les plus anciennes', () => {
  const storage = fakeStorage();
  for (let index = 0; index <= MAX_LAYOUT_GUILDS; index += 1) {
    writeLayoutCache(`g${index}`, [{ kind: 'image', width: 10, height: 10 }], storage, index);
  }
  assert.equal(layoutCacheSize(storage), MAX_LAYOUT_GUILDS);
  assert.equal(readLayoutCache('g0', storage), null);
  assert.notEqual(readLayoutCache(`g${MAX_LAYOUT_GUILDS}`, storage), null);
});

test('un JSON corrompu ou une autre version est ignoré', () => {
  assert.equal(readLayoutCache('g1', fakeStorage('{not json')), null);
  assert.equal(
    readLayoutCache('g1', fakeStorage(JSON.stringify({ version: 99, guilds: {} }))),
    null,
  );
});

test('un stockage qui refuse l’écriture ne casse pas le cache', () => {
  const storage = {
    getItem: () => null,
    setItem: () => {
      throw new Error('quota');
    },
    removeItem: () => {},
  };
  assert.equal(writeLayoutCache('g1', [{ kind: 'image', width: 1, height: 1 }], storage), false);
  assert.equal(readLayoutCache('g1', storage), null);
});

test('prune retire les guildes inconnues et clear vide tout', () => {
  const storage = fakeStorage();
  writeLayoutCache('g1', [{ kind: 'image', width: 10, height: 10 }], storage);
  writeLayoutCache('g2', [{ kind: 'video', width: 10, height: 10 }], storage);
  pruneLayoutCache(['g1'], storage);
  assert.equal(layoutCacheSize(storage), 1);
  assert.notEqual(readLayoutCache('g1', storage), null);
  assert.equal(readLayoutCache('g2', storage), null);
  clearLayoutCache(storage);
  assert.equal(layoutCacheSize(storage), 0);
});
