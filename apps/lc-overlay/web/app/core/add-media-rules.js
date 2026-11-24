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

// A link can always be downloaded again. A file is content-addressed, so the
// bypass only makes sense once its digest is known: the add dialog drives that
// case separately, after the file preflight.
export function canBypassServerCache(mode) {
  return mode === 'link';
}

// The bypass only makes sense when the server already holds the media: a new
// link has no cached copy to ignore. `cached` comes from the server lookup;
// a board hit is handled separately because the card itself proves the media
// exists without another round trip.
export function cacheBypassVisible(mode, cached) {
  return canBypassServerCache(mode) && cached === true;
}

export function forceRefreshFor(mode, checked) {
  return canBypassServerCache(mode) && checked === true;
}

// The state of the file preflight: a missing check is still being computed, so
// the submit must wait. `allOnBoard` is the only state that blocks an add: a
// cached upload can be imported without bytes, and an unknown file is uploaded.
export function filePreflightState(files, checks) {
  const list = Array.isArray(files) ? files : [];
  const entries = list.map((file) => checks.get(file));
  const checking = list.length > 0
    && entries.some((check) => !check || check.status === 'checking');
  const allOnBoard = list.length > 0
    && !checking
    && entries.every((check) => check && check.onBoard === true);
  const anyCached = entries.some((check) => check && (check.cached || check.onBoard));
  return { checking, allOnBoard, anyCached };
}

// The files that still need an upload. Adding a file already on the board
// would only re-pin its card, so it is skipped; a play-once may still play it,
// and the bypass keeps every file so the bytes are sent again.
export function filesToUpload(action, forceRefresh, files, checks) {
  const list = Array.isArray(files) ? files : [];
  if (action !== 'add' || forceRefresh === true) {
    return list.slice();
  }
  return list.filter((file) => {
    const check = checks.get(file);
    return !check || check.onBoard !== true;
  });
}

// The URL worth a link cache lookup: a trimmed http(s) link. File imports are
// keyed by their SHA-256 and use the upload preflight instead. The full
// validation (public host, credentials, length) belongs to the server.
export function cacheLookupUrl(mode, value) {
  if (mode !== 'link') {
    return null;
  }
  const raw = String(value ?? '').trim();
  if (!raw) {
    return null;
  }
  try {
    const url = new URL(raw);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    return raw;
  } catch {
    return null;
  }
}

// A source URL as the board stores it: http(s) only, trimmed, without its
// fragment. The query is kept: a TikTok clip id lives there.
function comparableSourceUrl(value) {
  const raw = String(value ?? '').trim();
  if (!raw) {
    return null;
  }
  try {
    const url = new URL(raw);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    url.hash = '';
    return url.href;
  } catch {
    return null;
  }
}

// The board item already showing `value`, when it is a link. Used by the add
// dialog before it submits: adding the same link again would be a no-op, so the
// submit stays disabled until a forced refresh is asked for. An uploaded media
// has no http source URL and never matches a typed link.
export function boardItemForUrl(items, value) {
  const wanted = comparableSourceUrl(value);
  if (!wanted || !Array.isArray(items)) {
    return null;
  }
  return items.find((item) => comparableSourceUrl(item && item.media && item.media.sourceUrl) === wanted) || null;
}

// An edited media is content-addressed (`upload:sha256:…`) but keeps the URL
// it was imported from. Adding that URL again is not the same media as the
// edited card: the server creates a second card with the original, so this
// must not be treated as a duplicate.
export function boardItemIsEdited(item) {
  return Boolean(item && item.media && item.media.edited === true);
}

// Whether the add submit must wait for the bypass: the media is already on the
// board, and the add would only re-pin the existing card without changing it.
// An edited card points at another asset, so the add creates the original as a
// second card and is allowed without the bypass. A forced refresh makes the
// submission meaningful again for a real duplicate. A play-once is not an add:
// it may play the card already there without touching the cache.
export function boardDuplicateNeedsRefresh(action, mode, item, forceRefresh) {
  return action === 'add'
    && mode === 'link'
    && Boolean(item)
    && !boardItemIsEdited(item)
    && forceRefresh !== true;
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
