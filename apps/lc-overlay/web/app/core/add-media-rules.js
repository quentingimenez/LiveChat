// Which action the add-media dialog starts on, and which fields it may show.
// A caption and a display time belong to a play once only: the server ignores
// them when the media is added to the board, so they must never be offered
// for an add and silently dropped at submit time.

export function defaultAction(canAdd, canPlay) {
  if (canAdd) {
    return 'add';
  }
  return canPlay ? 'play' : 'add';
}

// A link dropped on the board starts on the link source; files pasted from the
// clipboard start on the file source with their list already filled.
export function initialSource(files) {
  return files && files.length > 0 ? 'file' : 'link';
}

export function showsPlayFields(action, canPlay) {
  return Boolean(canPlay) && action === 'play';
}
