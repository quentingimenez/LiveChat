// Pure decisions of the board's media detail window (no DOM, unit-tested in
// dev/tests/detail-actions.test.mjs): whether the hover preview may show, the
// meta line it displays, and the actions the window offers.

// The preview only exists for the device setting « Ouvrir l'aperçu au survol »,
// and never over the selection mode, a shortcut capture or a dialog.
export function previewAllowed({ hoverPreview, selecting, capturing, dialogOpen } = {}) {
  return hoverPreview === true && !selecting && !capturing && !dialogOpen;
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
