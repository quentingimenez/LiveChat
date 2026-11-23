// A media drop: what a drag carries, and where it lands.
//
// The board owns the drag itself: it keeps the WebView from opening a dropped
// file anywhere in the window, shows the veil and decides the destination.
// While « Ajouter un média » is open, its dialog covers the board, so the drop
// fills the form instead of importing behind it (the filling is in
// views/add-media.js; the board hands it the transfer).

import { droppedLink } from './board-filters.js';

// The web link of a drag: a browser tab or a link dragged from a page writes it
// to `text/uri-list`, a text selection only to `text/plain`.
function transferLink(transfer) {
  if (!transfer || typeof transfer.getData !== 'function') {
    return null;
  }
  return droppedLink(transfer.getData('text/uri-list')) || droppedLink(transfer.getData('text/plain'));
}

// What a drop brings: its files, or else its link. Files win over the link — a
// drag carries one or the other — and a drag with neither brings no media.
export function droppedMedia(transfer) {
  const files = transfer && transfer.files ? Array.from(transfer.files) : [];
  if (files.length > 0) {
    return { files, link: null };
  }
  const link = transferLink(transfer);
  return link ? { files: [], link } : null;
}

// Where a drop may land: the open dialog first, since it covers the board, then
// the board's main area. `null` means nothing takes this drop.
export function dropTarget({ dialogOpen, onBoard }) {
  if (dialogOpen) {
    return 'dialog';
  }
  return onBoard ? 'board' : null;
}

// What the veil promises: the dialog's form when it covers the board, the board
// itself otherwise.
export const DROP_VEIL_TEXT = {
  board: 'Dépose un fichier ou un lien pour l’ajouter au board',
  dialog: 'Dépose un fichier ou un lien pour remplir le formulaire',
};
