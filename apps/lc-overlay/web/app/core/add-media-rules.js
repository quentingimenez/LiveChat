// Rules of the add-media dialog: which action it starts on, which fields it
// may show, and how the files picked for one import build up. A caption and a
// display time belong to a play once only: the server ignores them when the
// media is added to the board, so they must never be offered for an add and
// silently dropped at submit time.

export function defaultAction(canAdd, canPlay) {
  if (canAdd) {
    return 'add';
  }
  return canPlay ? 'play' : 'add';
}

export function showsPlayFields(action, canPlay) {
  return Boolean(canPlay) && action === 'play';
}

// The native picker replaces its own selection at each pick. The dialog keeps
// one import in progress, so a new pick is appended to the files already
// chosen, in the order they will be sent. A file picked twice is kept twice:
// the same media may legitimately join the board twice, and the cross on its
// row removes the extra one.
export function addPickedFiles(current, picked) {
  return current.concat(picked);
}

// Drops the file at `index`. A position that no longer exists (a stale render)
// drops nothing, and the caller always receives a fresh list.
export function removePickedFile(current, index) {
  if (!Number.isInteger(index) || index < 0 || index >= current.length) {
    return current.slice();
  }
  return current.slice(0, index).concat(current.slice(index + 1));
}
