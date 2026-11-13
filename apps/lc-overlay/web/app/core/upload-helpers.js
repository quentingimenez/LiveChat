// Pure helpers for media uploads. No imports: this module must also load under
// Node for its unit tests.

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

// Readable message for an upload rejected by the server. `labels` is the
// known error-label table (ERROR_LABELS), passed in to keep this module free of
// imports.
export function uploadErrorMessage(status, body, labels) {
  const code = body && body.error;
  if (status === 413) {
    return 'Fichier trop volumineux pour le serveur.';
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
  return status ? `Le serveur a répondu ${status}.` : 'L’envoi a été refusé.';
}
