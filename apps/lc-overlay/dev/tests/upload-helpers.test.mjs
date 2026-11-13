import assert from 'node:assert/strict';
import { test } from 'node:test';

import { chunkRanges, defaultTitle, uploadErrorMessage } from '../../web/app/core/upload-helpers.js';

test('chunkRanges: empty file', () => {
  assert.deepEqual(chunkRanges(0, 4), []);
});

test('chunkRanges: size of exactly one chunk', () => {
  assert.deepEqual(chunkRanges(4, 4), [[0, 4]]);
});

test('chunkRanges: several chunks with a partial last one', () => {
  assert.deepEqual(chunkRanges(10, 4), [
    [0, 4],
    [4, 8],
    [8, 10],
  ]);
});

test('chunkRanges: file smaller than one chunk', () => {
  assert.deepEqual(chunkRanges(3, 4), [[0, 3]]);
});

test('defaultTitle: keeps the name but strips the extension', () => {
  assert.equal(defaultTitle('clip.final.mp4'), 'clip.final');
});

test('defaultTitle: a leading dot is not an extension', () => {
  assert.equal(defaultTitle('.env'), '.env');
});

test('defaultTitle: no extension', () => {
  assert.equal(defaultTitle('son'), 'son');
});

const LABELS = { invalid_payload: 'Valeur invalide.' };

test('uploadErrorMessage: 413', () => {
  assert.equal(uploadErrorMessage(413, { error: 'payload_too_large' }, LABELS), 'Fichier trop volumineux pour le serveur.');
});

test('uploadErrorMessage: 415', () => {
  assert.equal(uploadErrorMessage(415, { error: 'unsupported_media' }, LABELS), 'Format de fichier non pris en charge.');
});

test('uploadErrorMessage: 422 uses the server message', () => {
  assert.equal(
    uploadErrorMessage(422, { error: 'media_ingestion_failed', message: 'Ce fichier est protégé.' }, LABELS),
    'Ce fichier est protégé.',
  );
});

test('uploadErrorMessage: 422 without a server message', () => {
  assert.equal(
    uploadErrorMessage(422, { error: 'media_ingestion_failed' }, LABELS),
    'Le serveur n’a pas pu convertir ce média.',
  );
});

test('uploadErrorMessage: 403 read_only_session has its own message', () => {
  assert.equal(uploadErrorMessage(403, { error: 'read_only_session' }, LABELS), 'Les invités ne peuvent pas ajouter de média.');
});

test('uploadErrorMessage: code known from the labels table', () => {
  assert.equal(uploadErrorMessage(400, { error: 'invalid_payload' }, LABELS), 'Valeur invalide.');
});

test('uploadErrorMessage: unknown status', () => {
  assert.equal(uploadErrorMessage(500, { error: 'teapot' }, LABELS), 'Le serveur a répondu 500.');
});
