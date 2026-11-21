// Badges de lecture des cartes du board (Node's built-in test runner).
//
// Ce que ce fichier protège : le badge « À l'écran » doit survivre à un
// changement de guilde. Après un aller-retour, la file serveur seule doit
// désigner la carte qui joue (même média asset que l'item), sans dépendre du
// clic qui a lancé le média ni d'un indice de session.

import assert from 'node:assert/strict';
import test from 'node:test';

import { badgeForItem } from '../../web/app/core/queue-badges.js';

const ON_AIR = { air: true, label: 'À l’écran' };
const job = (id, mediaAssetId = null, mediaId = null) => ({
  id,
  mediaAssetId,
  media: mediaId ? { id: mediaId } : null,
});
const item = (id, mediaAssetId = 'a0') => ({ id, mediaAssetId });
const queue = (overrides = {}) => ({ playing: null, pending: [], at: 1_000, ...overrides });
const noMap = () => new Map();

test('la file suffit à badge la carte du média qui joue', () => {
  const playing = queue({ playing: job('job-a', 'a0') });
  assert.deepEqual(badgeForItem(playing, noMap(), noMap(), item('item-a', 'a0')), ON_AIR);
});

test('la file peut nommer le média par son objet media', () => {
  const playing = queue({ playing: job('job-a', null, 'a0') });
  assert.deepEqual(badgeForItem(playing, noMap(), noMap(), item('item-a', 'a0')), ON_AIR);
});

test('une carte dont le média ne joue pas ne reçoit aucun badge', () => {
  const playing = queue({ playing: job('job-a', 'a0') });
  assert.equal(badgeForItem(playing, noMap(), noMap(), item('item-b', 'a1')), null);
});

test('la position en attente vient de la file serveur', () => {
  const pending = queue({
    pending: [job('job-b', 'a1'), job('job-a', 'a0'), job('job-c', 'a2')],
  });
  assert.deepEqual(badgeForItem(pending, noMap(), noMap(), item('item-a', 'a0')), {
    air: false,
    label: 'En file · 2',
  });
});

test('la carte qui joue prime sur une position en file', () => {
  const both = queue({ playing: job('job-a', 'a0'), pending: [job('job-b', 'a0')] });
  assert.deepEqual(badgeForItem(both, noMap(), noMap(), item('item-a', 'a0')), ON_AIR);
});

test('un job sans média ne badge aucune carte', () => {
  const playing = queue({ playing: job('job-a') });
  assert.equal(badgeForItem(playing, noMap(), noMap(), item('item-a', 'a0')), null);
});

test('le job cliqué reste reconnu quand la file ne nomme pas son média', () => {
  const playing = queue({ playing: job('job-a') });
  const triggers = new Map([['job-a', 'item-a']]);
  assert.deepEqual(badgeForItem(playing, triggers, noMap(), item('item-a', 'a0')), ON_AIR);
});

test("l'indice du clic badge la carte avant la première lecture de la file", () => {
  const hints = new Map([['job-a', { outcome: 'playing', at: 1_000 }]]);
  const triggers = new Map([['job-a', 'item-a']]);
  assert.deepEqual(
    badgeForItem(queue({ at: 900 }), triggers, hints, item('item-a', 'a0')),
    ON_AIR,
  );
});

test("l'indice du clic en file affiche la position annoncée", () => {
  const hints = new Map([['job-a', { outcome: 'queued', position: 3, at: 1_000 }]]);
  const triggers = new Map([['job-a', 'item-a']]);
  assert.deepEqual(badgeForItem(queue({ at: 900 }), triggers, hints, item('item-a', 'a0')), {
    air: false,
    label: 'En file · 3',
  });
});

test("l'indice du clic en file sans position reste lisible", () => {
  const hints = new Map([['job-a', { outcome: 'queued', position: null, at: 1_000 }]]);
  const triggers = new Map([['job-a', 'item-a']]);
  assert.deepEqual(badgeForItem(queue({ at: 900 }), triggers, hints, item('item-a', 'a0')), {
    air: false,
    label: 'En file',
  });
});

test('une lecture de file plus récente que le clic efface le badge', () => {
  const hints = new Map([['job-a', { outcome: 'queued', position: 3, at: 1_000 }]]);
  const triggers = new Map([['job-a', 'item-a']]);
  assert.equal(badgeForItem(queue({ at: 2_000 }), triggers, hints, item('item-a', 'a0')), null);
});

test('un item inconnu ou absent ne reçoit aucun badge', () => {
  const playing = queue({ playing: job('job-a', 'a0') });
  const triggers = new Map([['job-a', 'item-a']]);
  assert.equal(badgeForItem(playing, triggers, noMap(), item('item-z', 'a9')), null);
  assert.equal(badgeForItem(playing, triggers, noMap(), null), null);
  assert.equal(badgeForItem(null, triggers, noMap(), item('item-a', 'a0')), null);
});
