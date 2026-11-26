// Pure decisions of the board's media detail window (no DOM, unit-tested in
// dev/tests/detail-actions.test.mjs): whether the hover preview may show, the
// meta line it displays, and the actions the window offers.

// Two clicks closer together than this are one gesture. The preview uses it to
// tell a second click from a new one; a board card uses it to tell the second
// click of a double click from a new click on the window its first click opened.
export const DOUBLE_CLICK_WINDOW = 600;

// The preview only exists for the device setting « Ouvrir l'aperçu au survol »,
// and never over the selection mode, a shortcut capture, a dialog or an open
// card menu: the preview covers its card whole, so it hides the menu.
export function previewAllowed({ hoverPreview, selecting, capturing, dialogOpen, menuOpen } = {}) {
  return hoverPreview === true && !selecting && !capturing && !dialogOpen && !menuOpen;
}

// The preview's one meta line: kind, then the duration and the author when
// they are known. An empty part never leaves a stray separator.
export function previewMeta({ kindLabel, duration, author } = {}) {
  return [
    kindLabel,
    duration,
    author ? `par ${author}` : '',
  ].filter(Boolean).join(' · ');
}

// Which actions the information column offers, in display order. The origin
// link and the media's facts are shown in the column itself, so only the true
// commands remain; each appears with the right (or the source) that the drawer
// and the context menu already honoured.
export function detailActionIds({ canPlay, canOpenEditor, canRefresh, canImport, canDelete } = {}) {
  const ids = [];
  if (canPlay) {
    ids.push('play');
  }
  if (canOpenEditor) {
    ids.push('edit');
  }
  if (canRefresh) {
    ids.push('refresh');
  }
  if (canImport) {
    ids.push('import');
  }
  if (canDelete) {
    ids.push('delete');
  }
  return ids;
}
