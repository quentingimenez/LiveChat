// The shell command `media_download` reports failures with stable codes
// (`invalid_link`, `http_404`, `too_large`…) so its behaviour can be tested
// without reading sentences. This module is the only place where those codes
// become the text the user reads: an unknown or broken code never reaches a
// toast as is.

export const MEDIA_DOWNLOAD_MESSAGES = {
  invalidLink: 'Ce média n’a pas une adresse valide.',
  unreachable: 'Téléchargement impossible : le serveur est injoignable.',
  tooManyRedirects: 'Le lien du média redirige trop de fois.',
  tooLarge: 'Média trop volumineux pour un téléchargement local.',
  empty: 'Le serveur a renvoyé un média vide.',
  notMedia: 'Le serveur n’a pas renvoyé de média.',
  io: 'Impossible d’enregistrer le média à l’emplacement choisi.',
  unknown: 'Téléchargement du média impossible.',
};

const BY_CODE = new Map([
  ['invalid_link', MEDIA_DOWNLOAD_MESSAGES.invalidLink],
  ['unreachable', MEDIA_DOWNLOAD_MESSAGES.unreachable],
  ['too_many_redirects', MEDIA_DOWNLOAD_MESSAGES.tooManyRedirects],
  ['too_large', MEDIA_DOWNLOAD_MESSAGES.tooLarge],
  ['empty', MEDIA_DOWNLOAD_MESSAGES.empty],
  ['not_media', MEDIA_DOWNLOAD_MESSAGES.notMedia],
  ['io', MEDIA_DOWNLOAD_MESSAGES.io],
]);

// `invoke` rejects with the value the Rust command returned, here a string; an
// Error is accepted too, so the mapping does not depend on that detail.
export function mediaDownloadError(error) {
  const raw = typeof error === 'string' ? error : error instanceof Error ? error.message : '';
  const code = String(raw || '').trim();
  const http = /^http_(\d{3})$/.exec(code);
  if (http) {
    return `Le serveur a répondu par une erreur (HTTP ${Number(http[1])}).`;
  }
  return BY_CODE.get(code) || MEDIA_DOWNLOAD_MESSAGES.unknown;
}
