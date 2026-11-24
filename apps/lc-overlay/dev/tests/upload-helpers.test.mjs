import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  chunkRanges,
  defaultTitle,
  formatSizeLimit,
  ingestionErrorMessage,
  linkUploadBody,
  uploadErrorMessage,
  uploadStatusLabel,
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

test('uploadStatusLabel: waiting, sending and converting states', () => {
  assert.equal(uploadStatusLabel({ status: 'waiting', kind: 'link' }), 'En attente…');
  assert.equal(uploadStatusLabel({ status: 'sending', kind: 'link' }), 'Téléchargement…');
  assert.equal(uploadStatusLabel({ status: 'sending', kind: 'file' }), 'Envoi…');
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'file' }), 'Conversion…');
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'edit' }), 'Montage…');
});

test('uploadStatusLabel: a play-once says it is being read, whatever the kind', () => {
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'link', playOnce: true }), 'Lecture…');
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'file', playOnce: true }), 'Lecture…');
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'edit', playOnce: true }), 'Lecture…');
  // The transfer states keep their own wording.
  assert.equal(uploadStatusLabel({ status: 'waiting', kind: 'file', playOnce: true }), 'En attente…');
  assert.equal(uploadStatusLabel({ status: 'sending', kind: 'file', playOnce: true }), 'Envoi…');
  assert.equal(uploadStatusLabel({ status: 'sending', kind: 'link', playOnce: true }), 'Téléchargement…');
  // Without playOnce, the conversion keeps its wording.
  assert.equal(uploadStatusLabel({ status: 'converting', kind: 'file', playOnce: false }), 'Conversion…');
});

const LABELS = { invalid_payload: 'Valeur invalide.' };

test('uploadStatusLabel: a refresh of an existing card says so', () => {
  assert.equal(
    uploadStatusLabel({ status: 'sending', kind: 'link', refreshItemId: 'item-1' }),
    'Rafraîchissement…',
  );
  assert.equal(
    uploadStatusLabel({ status: 'converting', kind: 'link', refreshItemId: 'item-1' }),
    'Rafraîchissement…',
  );
  // Queued or failed, it is still the ordinary upload wording.
  assert.equal(
    uploadStatusLabel({ status: 'waiting', kind: 'link', refreshItemId: 'item-1' }),
    'En attente…',
  );
  assert.equal(uploadStatusLabel({ status: 'sending', kind: 'link' }), 'Téléchargement…');
});

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

test('uploadErrorMessage: another 429 is explained as a rate limit', () => {
  assert.equal(
    uploadErrorMessage(429, { error: 'request_error' }, LABELS),
    'Trop de requêtes vers le serveur (429) : réessaie dans quelques minutes.',
  );
});

test('uploadErrorMessage: gateway failures say the server is unreachable', () => {
  for (const status of [502, 503, 504]) {
    assert.equal(uploadErrorMessage(status, null, LABELS), `Serveur injoignable (${status}) : réessaie dans un instant.`);
  }
});

test('uploadErrorMessage: 413 names the server limit when it is known', () => {
  const body = { error: 'payload_too_large', maxBytes: 2048 * 1024 * 1024 };
  assert.equal(uploadErrorMessage(413, body, LABELS), 'Fichier trop volumineux : 2 Go maximum.');
});

test('uploadErrorMessage: too_many_uploads', () => {
  assert.equal(
    uploadErrorMessage(429, { error: 'too_many_uploads' }, LABELS),
    'Trop d’envois en cours. Réessaie dans un instant.',
  );
});

test('formatSizeLimit: mebibytes under 1 GiB, gibibytes above', () => {
  assert.equal(formatSizeLimit(500 * 1024 * 1024), '500 Mo');
  assert.equal(formatSizeLimit(1024 * 1024 * 1024), '1 Go');
  assert.equal(formatSizeLimit(1536 * 1024 * 1024), '1,5 Go');
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

test('linkUploadBody: un ajout simple ne porte pas forceRefresh', () => {
  const body = linkUploadBody({ url: 'https://example.test/clip', title: 'Clip' });

  assert.deepEqual(body, { url: 'https://example.test/clip', title: 'Clip' });
  assert.equal('forceRefresh' in body, false);
});

test('linkUploadBody: le contournement du cache est transmis au board', () => {
  const body = linkUploadBody({ url: 'https://example.test/clip', title: 'Clip', forceRefresh: true });

  assert.equal(body.forceRefresh, true);
});

test('linkUploadBody: une case décochée ou un fichier ne fait pas partir le drapeau', () => {
  assert.equal('forceRefresh' in linkUploadBody({ url: 'https://example.test/clip', title: 'Clip', forceRefresh: false }), false);
  assert.equal('forceRefresh' in linkUploadBody({ url: 'https://example.test/clip', title: 'Clip' }), false);
});

test('linkUploadBody: un jouer une fois garde sa légende, sa durée et le contournement', () => {
  const body = linkUploadBody({
    url: 'https://example.test/clip',
    title: 'Clip',
    playOnce: true,
    message: 'Légende',
    displayDurationSec: 12,
    forceRefresh: true,
  });

  assert.deepEqual(body, {
    url: 'https://example.test/clip',
    title: 'Clip',
    playOnce: true,
    message: 'Légende',
    displayDurationSec: 12,
    forceRefresh: true,
  });
});
