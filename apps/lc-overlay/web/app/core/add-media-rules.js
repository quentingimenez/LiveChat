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

// A link dropped on the board starts on the link source; files pasted from the
// clipboard start on the file source with their list already filled.
export function initialSource(files) {
  return files && files.length > 0 ? 'file' : 'link';
}

export function showsPlayFields(action, canPlay) {
  return Boolean(canPlay) && action === 'play';
}

// Two picks of the same file hand back two File objects: it is one media, and
// the import must not list it twice. Name, size, type and last modified date
// are what the picker gives to tell one file from another.
export function samePickedFile(a, b) {
  return a.name === b.name
    && a.size === b.size
    && a.type === b.type
    && a.lastModified === b.lastModified;
}

// The native picker replaces its own selection at each pick. The dialog keeps
// one import in progress, so a new pick is appended to the files already
// chosen, in the order they will be sent — without the files already listed: a
// media listed twice would be sent twice to the board. The duplicates of one
// single pick are dropped the same way.
export function addPickedFiles(current, picked) {
  const next = current.slice();
  for (const file of picked) {
    if (!next.some((kept) => samePickedFile(kept, file))) {
      next.push(file);
    }
  }
  return next;
}

// A pick that added nothing must say why: the message counts the media the
// import already had, so the button does not look broken.
export function duplicatePickMessage(count) {
  return count > 1 ? `${count} médias déjà dans l’importation.` : 'Média déjà dans l’importation.';
}

// Drops the file at `index`. A position that no longer exists (a stale render)
// drops nothing, and the caller always receives a fresh list.
export function removePickedFile(current, index) {
  if (!Number.isInteger(index) || index < 0 || index >= current.length) {
    return current.slice();
  }
  return current.slice(0, index).concat(current.slice(index + 1));
}
