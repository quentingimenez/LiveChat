// Picture of an audio media: what can be used as one (a chosen, dropped or
// pasted image file, or the link of an image downloaded in the WebView).

import { droppedLink } from './board-filters.js';

export const MAX_ARTWORK_BYTES = 10 * 1024 * 1024;

export const ARTWORK_MESSAGES = {
  tooLarge: 'Image trop volumineuse (10 Mo au plus).',
  notImageFile: 'Ce fichier n’est pas une image.',
  notImageLink: 'Ce lien ne renvoie pas une image.',
  invalidLink: 'Ce lien n’est pas une adresse http(s) valide.',
  empty: 'Ce lien renvoie une image vide.',
  // A cross-origin download refused by the site (CORS) and a network failure
  // look the same to a page: say both, and what still works.
  unreachable:
    'Téléchargement impossible : le site refuse l’accès depuis l’application (CORS) ou est injoignable. Enregistre l’image puis dépose le fichier.',
  nothingDropped: 'Dépose une image ou le lien d’une image.',
};

export function httpErrorMessage(status) {
  return `Le lien a répondu par une erreur (HTTP ${status}).`;
}

// Media type without its parameters, lower-cased (`image/png`).
export function baseType(type) {
  return typeof type === 'string' ? type.split(';')[0].trim().toLowerCase() : '';
}

export function isImageType(type) {
  return baseType(type).startsWith('image/');
}

// Extension sent with the picture: the one of the file name, else the one of
// its media type.
export function artworkExtension(file) {
  const match = /\.([a-z0-9]{1,5})$/i.exec(file.name || '');
  if (match) {
    return `.${match[1].toLowerCase()}`;
  }
  const subtype = /^image\/([a-z0-9]+)/i.exec(file.type || '');
  return subtype ? `.${subtype[1].toLowerCase().replace('jpeg', 'jpg')}` : '';
}

// The picture among chosen, dropped or pasted files: `{ file }`, `{ error }`
// or `null` when there is no file at all.
export function pickArtworkFile(files) {
  const list = Array.from(files || []);
  if (list.length === 0) {
    return null;
  }
  const file = list.find((entry) => isImageType(entry.type));
  if (!file) {
    return { error: ARTWORK_MESSAGES.notImageFile };
  }
  if (file.size > MAX_ARTWORK_BYTES) {
    return { error: ARTWORK_MESSAGES.tooLarge };
  }
  return { file };
}

// First `src` of an `<img>` in a dragged HTML fragment.
function imageSourceOf(html) {
  if (typeof html !== 'string') {
    return null;
  }
  const match = /<img\b[^>]*?\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i.exec(html);
  if (!match) {
    return null;
  }
  const raw = match[1] ?? match[2] ?? match[3] ?? '';
  return droppedLink(raw.replace(/&amp;/g, '&'));
}

// The link of a drop that carries no file. An image dragged from a web page
// comes with its HTML: its `src` is the picture, while the URI list may be
// the page the image links to.
export function droppedArtworkLink({ html, uriList, text } = {}) {
  return imageSourceOf(html) || droppedLink(uriList) || droppedLink(text) || null;
}

// File name given to a downloaded picture: the one in the link when it has an
// extension, else `image` with the extension of the media type.
export function artworkFileName(url, type) {
  let name = '';
  try {
    name = decodeURIComponent(new URL(url).pathname.split('/').pop() || '');
  } catch {
    name = '';
  }
  // Path separators and control characters never reach a file name.
  name = name.replace(/[\\/:*?"<>|\u0000-\u001f]/g, '').trim();
  if (/\.[a-z0-9]{1,5}$/i.test(name)) {
    return name;
  }
  const extension = artworkExtension({ name: '', type: baseType(type) });
  return `${name || 'image'}${extension}`;
}

// Reads a response body, giving up as soon as it grows over `limit` bytes.
// Returns the chunks, or `null` when the body is too large.
async function readCapped(response, limit) {
  if (!response.body || typeof response.body.getReader !== 'function') {
    const blob = await response.blob();
    return blob.size > limit ? null : [blob];
  }
  const reader = response.body.getReader();
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      return chunks;
    }
    size += value.byteLength;
    if (size > limit) {
      await reader.cancel().catch(() => {});
      return null;
    }
    chunks.push(value);
  }
}

// Downloads the image behind a link. Returns `{ file }` (a File ready for the
// upload path) or `{ error }` with a message for the user.
export async function fetchArtwork(link, fetchImpl = globalThis.fetch) {
  const url = droppedLink(typeof link === 'string' ? link : '');
  if (!url) {
    return { error: ARTWORK_MESSAGES.invalidLink };
  }
  let response;
  try {
    response = await fetchImpl(url, { credentials: 'omit', referrerPolicy: 'no-referrer' });
  } catch {
    return { error: ARTWORK_MESSAGES.unreachable };
  }
  if (!response.ok) {
    return { error: httpErrorMessage(response.status) };
  }
  const type = baseType(response.headers.get('content-type'));
  if (!isImageType(type)) {
    return { error: ARTWORK_MESSAGES.notImageLink };
  }
  if (Number(response.headers.get('content-length')) > MAX_ARTWORK_BYTES) {
    return { error: ARTWORK_MESSAGES.tooLarge };
  }
  let chunks;
  try {
    chunks = await readCapped(response, MAX_ARTWORK_BYTES);
  } catch {
    return { error: ARTWORK_MESSAGES.unreachable };
  }
  if (!chunks) {
    return { error: ARTWORK_MESSAGES.tooLarge };
  }
  const file = new File(chunks, artworkFileName(url, type), { type });
  if (file.size === 0) {
    return { error: ARTWORK_MESSAGES.empty };
  }
  return { file };
}
