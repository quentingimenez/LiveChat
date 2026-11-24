// The add-media dialog has two actions with different fields: a caption and a
// display time belong to « Jouer une fois » only. The server ignores them for
// an add, so they must never be offered for it.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  boardDuplicateNeedsRefresh,
  boardItemForUrl,
  boardItemIsEdited,
  cacheBypassVisible,
  cacheLookupUrl,
  canBypassServerCache,
  defaultAction,
  filePreflightState,
  filesToUpload,
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

test('le contournement d’un lien passe par la règle du lien', () => {
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

test('forceRefreshFor reste la règle du lien ; un fichier force son envoi directement', () => {
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

test('la board reconnaît un lien déjà présent, sans confondre deux médias', () => {
  const item = { id: 'item-1', media: { sourceUrl: 'https://example.test/watch?v=2#part' } };
  const items = [
    item,
    { id: 'item-2', media: { sourceUrl: null } },
    { id: 'item-3', media: { sourceUrl: 'https://example.test/other' } },
  ];
  assert.equal(boardItemForUrl(items, ' https://example.test/watch?v=2 '), item);
  assert.equal(boardItemForUrl(items, 'https://example.test/other'), items[2]);
  assert.equal(boardItemForUrl(items, 'https://example.test/watch?v=3'), null);
  assert.equal(boardItemForUrl(items, 'file:///tmp/clip.mp4'), null);
  assert.equal(boardItemForUrl(items, ''), null);
  assert.equal(boardItemForUrl(null, 'https://example.test/watch?v=2'), null);
});

test('un média déjà sur la board bloque l’envoi sauf avec le contournement', () => {
  const item = { id: 'item-1', media: { sourceUrl: 'https://example.test/watch?v=2' } };
  assert.equal(boardDuplicateNeedsRefresh('add', 'link', item, false), true);
  assert.equal(boardDuplicateNeedsRefresh('add', 'link', item, true), false);
  assert.equal(boardDuplicateNeedsRefresh('play', 'link', item, false), false);
  assert.equal(boardDuplicateNeedsRefresh('add', 'link', null, false), false);
  assert.equal(boardDuplicateNeedsRefresh('add', 'file', item, false), false);
});

test('le préflight fichier distingue vérification, board et cache serveur', () => {
  const first = { name: 'first.mp4' };
  const second = { name: 'second.mp4' };
  const checks = new Map();
  assert.deepEqual(filePreflightState([], checks), {
    checking: false,
    allOnBoard: false,
    anyCached: false,
  });
  assert.deepEqual(filePreflightState([first], checks), {
    checking: true,
    allOnBoard: false,
    anyCached: false,
  });
  checks.set(first, { status: 'checked', cached: true, onBoard: false });
  assert.deepEqual(filePreflightState([first, second], checks), {
    checking: true,
    allOnBoard: false,
    anyCached: true,
  });
  checks.set(second, { status: 'checked', cached: false, onBoard: true });
  assert.deepEqual(filePreflightState([first, second], checks), {
    checking: false,
    allOnBoard: false,
    anyCached: true,
  });
  checks.set(first, { status: 'checked', cached: false, onBoard: true });
  assert.deepEqual(filePreflightState([first, second], checks), {
    checking: false,
    allOnBoard: true,
    anyCached: true,
  });
});

test('un ajout ne renvoie pas un fichier déjà sur la board, sauf avec le contournement', () => {
  const onboard = { name: 'onboard.mp4' };
  const cached = { name: 'cached.mp4' };
  const unknown = { name: 'unknown.mp4' };
  const checks = new Map([
    [onboard, { status: 'checked', cached: false, onBoard: true }],
    [cached, { status: 'checked', cached: true, onBoard: false }],
  ]);
  assert.deepEqual(filesToUpload('add', false, [onboard, cached, unknown], checks), [cached, unknown]);
  assert.deepEqual(filesToUpload('add', true, [onboard, cached, unknown], checks), [onboard, cached, unknown]);
  assert.deepEqual(filesToUpload('play', false, [onboard, cached], checks), [onboard, cached]);
  assert.deepEqual(filesToUpload('add', false, [unknown], checks), [unknown]);
});

test('un média édité garde son URL mais ne bloque pas le réimport de l’original', () => {
  const item = {
    id: 'item-1',
    media: {
      sourceUrl: 'https://example.test/watch?v=2',
      edited: true,
    },
  };
  assert.equal(boardItemIsEdited(item), true);
  assert.equal(boardItemIsEdited({ id: 'item-2', media: { sourceUrl: 'https://example.test/x' } }), false);
  assert.equal(boardItemIsEdited(null), false);
  assert.equal(boardItemForUrl([item], 'https://example.test/watch?v=2'), item);
  // The edited card and the URL are different assets: a normal add creates the
  // original as a second card instead of re-pinning the edited one.
  assert.equal(boardDuplicateNeedsRefresh('add', 'link', item, false), false);
  assert.equal(boardDuplicateNeedsRefresh('add', 'link', item, true), false);
});
