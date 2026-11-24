// The add-media dialog has two actions with different fields: a caption and a
// display time belong to « Jouer une fois » only. The server ignores them for
// an add, so they must never be offered for it.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  cacheBypassVisible,
  cacheLookupUrl,
  canBypassServerCache,
  defaultAction,
  forceRefreshFor,
  initialSource,
  showsPlayFields,
} from '../../web/app/core/add-media-rules.js';

test('l’ajout au board est l’action par défaut quand on en a le droit', () => {
  assert.equal(defaultAction(true, true), 'add');
  assert.equal(defaultAction(true, false), 'add');
});

test('un membre qui ne peut qu’ajouter ne voit pas « Jouer une fois »', () => {
  assert.equal(defaultAction(true, false), 'add');
});

test('un membre qui ne peut que jouer arrive sur « Jouer une fois »', () => {
  assert.equal(defaultAction(false, true), 'play');
});

test('la légende et la durée ne s’affichent que pour « Jouer une fois »', () => {
  assert.equal(showsPlayFields('add', true), false);
  assert.equal(showsPlayFields('play', true), true);
  assert.equal(showsPlayFields('play', false), false);
  assert.equal(showsPlayFields('add', false), false);
});

test('des fichiers collés ouvrent la source Fichier, sinon la source Lien', () => {
  assert.equal(initialSource([]), 'link');
  assert.equal(initialSource([{ name: 'photo.jpg', type: 'image/jpeg', size: 11748 }]), 'file');
});

test('le cache serveur ne se contourne que pour un lien', () => {
  assert.equal(canBypassServerCache('link'), true);
  assert.equal(canBypassServerCache('file'), false);
  assert.equal(canBypassServerCache(''), false);
});

test('le contournement ne s’affiche que si le média est déjà connu', () => {
  assert.equal(cacheBypassVisible('link', true), true);
  assert.equal(cacheBypassVisible('link', false), false);
  assert.equal(cacheBypassVisible('file', true), false);
  assert.equal(cacheBypassVisible('file', false), false);
});

test('un fichier ne demande jamais le contournement, même si la case reste cochée', () => {
  assert.equal(forceRefreshFor('link', true), true);
  assert.equal(forceRefreshFor('link', false), false);
  assert.equal(forceRefreshFor('file', true), false);
  assert.equal(forceRefreshFor('file', false), false);
  assert.equal(forceRefreshFor(undefined, true), false);
});

test('seul un lien http(s) est interrogé auprès du cache serveur', () => {
  assert.equal(
    cacheLookupUrl('link', ' https://example.test/watch?v=2 '),
    'https://example.test/watch?v=2',
  );
  assert.equal(cacheLookupUrl('link', 'http://example.test/clip'), 'http://example.test/clip');
  assert.equal(cacheLookupUrl('link', 'https://example.test/clip#cover'), 'https://example.test/clip#cover');
  assert.equal(cacheLookupUrl('file', 'https://example.test/clip'), null);
  assert.equal(cacheLookupUrl('link', 'ftp://example.test/clip'), null);
  assert.equal(cacheLookupUrl('link', 'pas une url'), null);
  assert.equal(cacheLookupUrl('link', '  '), null);
});
