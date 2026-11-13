import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  MAX_UPLOAD_BYTES,
  chunkRanges,
  defaultTitle,
  ingestionErrorMessage,
  isUploadTooLarge,
  uploadErrorMessage,
} from '../../web/app/core/upload-helpers.js';

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

test('uploadErrorMessage: 429 too_many_ingestions', () => {
  assert.equal(
    uploadErrorMessage(429, { error: 'too_many_ingestions' }, LABELS),
    'Trop de médias en cours de conversion. Réessaie dans un instant.',
  );
});

test('uploadErrorMessage: another 429 keeps the generic message', () => {
  assert.equal(uploadErrorMessage(429, { error: 'rate_limited' }, LABELS), 'Le serveur a répondu 429.');
});

test('MAX_UPLOAD_BYTES is 100 MiB', () => {
  assert.equal(MAX_UPLOAD_BYTES, 100 * 1024 * 1024);
});

test('isUploadTooLarge: exactly 100 MiB is accepted', () => {
  assert.equal(isUploadTooLarge(MAX_UPLOAD_BYTES), false);
});

test('isUploadTooLarge: one byte over the limit is refused', () => {
  assert.equal(isUploadTooLarge(MAX_UPLOAD_BYTES + 1), true);
});

test('isUploadTooLarge: non-numeric sizes are not considered too large', () => {
  assert.equal(isUploadTooLarge(undefined), false);
  assert.equal(isUploadTooLarge('not-a-number'), false);
});

test('ingestionErrorMessage: unsupported_media', () => {
  assert.equal(
    ingestionErrorMessage({ code: 'unsupported_media', message: 'mkv' }, LABELS),
    'Format de fichier non pris en charge.',
  );
});

test('ingestionErrorMessage: media_ingestion_failed uses the server message', () => {
  assert.equal(
    ingestionErrorMessage({ code: 'media_ingestion_failed', message: 'Ce fichier est protégé.' }, LABELS),
    'Ce fichier est protégé.',
  );
});

test('ingestionErrorMessage: media_ingestion_failed without a message', () => {
  assert.equal(
    ingestionErrorMessage({ code: 'media_ingestion_failed' }, LABELS),
    'Le serveur n’a pas pu convertir ce média.',
  );
});

test('ingestionErrorMessage: code known from the labels table', () => {
  assert.equal(ingestionErrorMessage({ code: 'invalid_payload', message: 'détail' }, LABELS), 'Valeur invalide.');
});

test('ingestionErrorMessage: unknown code falls back to the server message', () => {
  assert.equal(ingestionErrorMessage({ code: 'something_new', message: 'détail serveur' }, LABELS), 'détail serveur');
});

test('ingestionErrorMessage: unknown code without a message', () => {
  assert.equal(ingestionErrorMessage({ code: 'something_new' }, LABELS), 'La conversion a échoué.');
  assert.equal(ingestionErrorMessage(null, LABELS), 'La conversion a échoué.');
});
