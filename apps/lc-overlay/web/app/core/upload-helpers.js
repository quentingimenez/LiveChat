// Pure helpers for media uploads. Only pure modules are imported: this one
// must also load under Node for its unit tests.

import { serverStatusMessage } from './http-status.js';

// Splits a byte size into [start, end) ranges sent one after the other.
export function chunkRanges(size, chunkSize) {
  const total = Number(size);
  const step = Number(chunkSize);
  if (!Number.isFinite(total) || !Number.isFinite(step) || total <= 0 || step <= 0) {
    return [];
  }
  const ranges = [];
  for (let start = 0; start < total; start += step) {
    ranges.push([start, Math.min(start + step, total)]);
  }
  return ranges;
}

// Default title of an uploaded file: its name without the extension. A leading
// dot does not make an extension (".env" keeps its name).
export function defaultTitle(filename) {
  const name = String(filename ?? '');
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(0, dot) : name;
}

// Status label of a provisional upload card. A play-once says "Lecture…"
// while it is prepared: nothing will join the board.
export function uploadStatusLabel(entry) {
  if (entry.status === 'sending') {
    return entry.kind === 'link' ? 'Téléchargement…' : 'Envoi…';
  }
  if (entry.status === 'converting') {
    if (entry.playOnce) {
      return 'Lecture…';
    }
    return entry.kind === 'edit' ? 'Montage…' : 'Conversion…';
  }
  return 'En attente…';
}

// A byte limit in mebibytes, or gibibytes from 1 Gio: "500 Mo", "2 Go".
export function formatSizeLimit(bytes) {
  const mebibytes = Number(bytes) / (1024 * 1024);
  if (mebibytes >= 1024) {
    return `${Math.round((mebibytes / 1024) * 10) / 10} Go`.replace('.', ',');
  }
  return `${Math.round(mebibytes)} Mo`;
}

// Readable message for an upload rejected by the server. `labels` is the
// known error-label table (ERROR_LABELS), passed in to keep this module free of
// imports.
export function uploadErrorMessage(status, body, labels) {
  const code = body && body.error;
  if (status === 429 && code === 'too_many_ingestions') {
    return 'Trop de médias en cours de conversion. Réessaie dans un instant.';
  }
  if (status === 429 && code === 'too_many_uploads') {
    return 'Trop d’envois en cours. Réessaie dans un instant.';
  }
  if (status === 413) {
    const limit = Number(body && body.maxBytes);
    return Number.isFinite(limit) && limit > 0
      ? `Fichier trop volumineux : ${formatSizeLimit(limit)} maximum.`
      : 'Fichier trop volumineux pour le serveur.';
  }
  if (status === 415) {
    return 'Format de fichier non pris en charge.';
  }
  if (status === 422) {
    return (body && body.message) || 'Le serveur n’a pas pu convertir ce média.';
  }
  if (status === 403 && code === 'read_only_session') {
    return 'Les invités ne peuvent pas ajouter de média.';
  }
  const known = labels && code ? labels[code] : null;
  if (known) {
    return known;
  }
  return status ? serverStatusMessage(status) : 'L’envoi a été refusé.';
}

// Readable message for a failed asynchronous conversion. `labels` is the known
// error-label table, passed in to keep this module free of imports.
export function ingestionErrorMessage(error, labels) {
  const code = error && error.code;
  if (code === 'unsupported_media') {
    return 'Format de fichier non pris en charge.';
  }
  if (code === 'media_ingestion_failed') {
    return (error && error.message) || 'Le serveur n’a pas pu convertir ce média.';
  }
  const known = labels && code ? labels[code] : null;
  if (known) {
    return known;
  }
  return (error && error.message) || 'La conversion a échoué.';
}
