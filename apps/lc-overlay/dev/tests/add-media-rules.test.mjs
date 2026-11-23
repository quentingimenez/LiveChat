// The add-media dialog has two actions with different fields: a caption and a
// display time belong to « Jouer une fois » only. The server ignores them for
// an add, so they must never be offered for it.

import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultAction, initialSource, showsPlayFields } from '../../web/app/core/add-media-rules.js';

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
