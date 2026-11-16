import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  SAY_CAPTION_MAX_CHARS,
  SAY_MAX_CHARS,
  isSaySubmitKey,
  normalizeSpeech,
  sayErrorMessage,
  sayPayload,
  speechCounter,
  speechLength,
} from '../../web/app/core/say-helpers.js';

test('SAY_MAX_CHARS matches the server limit', () => {
  assert.equal(SAY_MAX_CHARS, 1000);
  assert.equal(SAY_CAPTION_MAX_CHARS, 500);
});

test('normalizeSpeech: collapses whitespace and trims', () => {
  assert.equal(normalizeSpeech('  bonjour \n  tout\t le   monde '), 'bonjour tout le monde');
  assert.equal(normalizeSpeech(' \n\t '), '');
  assert.equal(normalizeSpeech(undefined), '');
});

test('speechLength: counts characters, not UTF-16 units', () => {
  assert.equal(speechLength('été'), 3);
  assert.equal(speechLength('😀😀'), 2);
  assert.equal(speechLength('  a   b  '), 3);
});

test('speechCounter: empty text', () => {
  assert.deepEqual(speechCounter('   '), { length: 0, max: 1000, label: '0 / 1000', empty: true, tooLong: false, valid: false });
});

test('speechCounter: exactly the limit is valid', () => {
  const counter = speechCounter('a'.repeat(SAY_MAX_CHARS));
  assert.equal(counter.valid, true);
  assert.equal(counter.tooLong, false);
  assert.equal(counter.label, '1000 / 1000');
});

test('speechCounter: one character over the limit', () => {
  const counter = speechCounter('a'.repeat(SAY_MAX_CHARS + 1));
  assert.equal(counter.valid, false);
  assert.equal(counter.tooLong, true);
  assert.equal(counter.label, '1001 / 1000');
});

test('speechCounter: surrounding whitespace does not count', () => {
  assert.equal(speechCounter(`  ${'a'.repeat(SAY_MAX_CHARS)} \n`).valid, true);
});

test('sayPayload: text only', () => {
  assert.deepEqual(sayPayload({ text: ' Salut  à tous ' }), { text: 'Salut à tous' });
});

test('sayPayload: caption and anonymous', () => {
  assert.deepEqual(sayPayload({ text: 'Salut', caption: '  coucou ', anonymous: true }), {
    text: 'Salut',
    caption: 'coucou',
    anonymous: true,
  });
});

test('sayPayload: blank caption and unchecked anonymous are left out', () => {
  assert.deepEqual(sayPayload({ text: 'Salut', caption: '   ', anonymous: false }), { text: 'Salut' });
});

test('sayPayload: a long caption is cut', () => {
  const payload = sayPayload({ text: 'Salut', caption: 'é'.repeat(SAY_CAPTION_MAX_CHARS + 20) });
  assert.equal(Array.from(payload.caption).length, SAY_CAPTION_MAX_CHARS);
});

test('sayPayload: empty or too long text gives null', () => {
  assert.equal(sayPayload({ text: '  ' }), null);
  assert.equal(sayPayload({ text: 'a'.repeat(SAY_MAX_CHARS + 1) }), null);
  assert.equal(sayPayload(), null);
});

test('sayErrorMessage: known codes are in French', () => {
  assert.equal(sayErrorMessage(400, 'text_too_long'), 'Texte trop long : 1000 caractères au maximum.');
  assert.equal(sayErrorMessage(400, 'invalid_payload'), 'Écris le texte à faire dire.');
  assert.equal(
    sayErrorMessage(502, 'speech_failed', 'Serveur injoignable (502) : réessaie dans un instant.'),
    'La synthèse vocale a échoué. Réessaie avec un texte plus court ou plus tard.',
  );
  assert.equal(sayErrorMessage(403, 'read_only_session'), 'Les invités ne peuvent pas faire parler le bot.');
  assert.equal(sayErrorMessage(403, 'forbidden'), 'Ton rôle ne permet pas de faire parler le bot.');
});

test('sayErrorMessage: unknown code keeps the message already built', () => {
  assert.equal(sayErrorMessage(401, 'unauthorized', 'Cet appareil est inconnu.'), 'Cet appareil est inconnu.');
});

test('sayErrorMessage: no code and no message falls back to the status', () => {
  assert.equal(sayErrorMessage(429), 'Trop de requêtes vers le serveur (429) : réessaie dans quelques minutes.');
  assert.equal(sayErrorMessage(undefined), 'La demande a été refusée.');
});

test('isSaySubmitKey: Enter sends, Shift+Enter and composition do not', () => {
  assert.equal(isSaySubmitKey({ key: 'Enter' }), true);
  assert.equal(isSaySubmitKey({ key: 'Enter', shiftKey: true }), false);
  assert.equal(isSaySubmitKey({ key: 'Enter', ctrlKey: true }), false);
  assert.equal(isSaySubmitKey({ key: 'Enter', isComposing: true }), false);
  assert.equal(isSaySubmitKey({ key: 'a' }), false);
  assert.equal(isSaySubmitKey(undefined), false);
});
