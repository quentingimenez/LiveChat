// Which media the board's detail drawer shows, as pure transitions.
//
// A view is `{ selectedId, pinned, peekId }`:
// - `selectedId`: the media the drawer belongs to (pinned by a click, or just
//   followed by the pointer while nothing is pinned);
// - `pinned`: a click (or using the drawer) pinned `selectedId`;
// - `peekId`: another media looked at by hovering its card while one is
//   pinned. The drawer returns to the pinned media when the pointer leaves.

export const CLOSED_VIEW = Object.freeze({ selectedId: null, pinned: false, peekId: null });

// The media the drawer shows right now.
export function shownId(view) {
  return view.pinned && view.peekId ? view.peekId : view.selectedId;
}

// True while the drawer shows a media that is not pinned.
export function isPeeking(view) {
  return !view.pinned || Boolean(view.peekId);
}

export function sameView(a, b) {
  return a.selectedId === b.selectedId && a.pinned === b.pinned && a.peekId === b.peekId;
}

// A click on a card pins its media; a click on the pinned one closes the
// drawer. `clickCount` is the click's `event.detail`: the later clicks of a
// double click (which plays the media) leave the drawer as it is. The board
// holds back the first click until it knows it is not part of a double click.
export function cardClicked(view, id, clickCount = 1) {
  if (clickCount >= 2) {
    return view;
  }
  if (view.pinned && view.selectedId === id) {
    return { ...CLOSED_VIEW };
  }
  return { selectedId: id, pinned: true, peekId: null };
}

// The pointer stayed on a card for the hover delay.
export function cardHovered(view, id) {
  if (!view.pinned) {
    return { selectedId: id, pinned: false, peekId: null };
  }
  // Back on the pinned card: the peek ends.
  return { ...view, peekId: id === view.selectedId ? null : id };
}

// The pointer left the cards and the drawer for the leave delay.
export function pointerLeft(view) {
  if (!view.pinned) {
    return { ...CLOSED_VIEW };
  }
  return { ...view, peekId: null };
}

// Using the drawer keeps what it shows.
export function drawerUsed(view) {
  const id = shownId(view);
  return id ? { selectedId: id, pinned: true, peekId: null } : { ...CLOSED_VIEW };
}

// Drops a peek that cannot be shown any more (its media left the board, or
// nothing is pinned): the drawer goes back to the selected media.
export function pruneView(view, exists) {
  if (view.peekId && (!view.pinned || !exists(view.peekId))) {
    return { ...view, peekId: null };
  }
  return view;
}

// --- preview of the shown media ---

// Path of a signed media URL, without its query: the same asset keeps the
// same path while its signature changes every 15 minutes.
export function mediaPath(url) {
  if (typeof url !== 'string' || !url) {
    return null;
  }
  try {
    const parsed = new URL(url);
    return `${parsed.origin}${parsed.pathname}`;
  } catch {
    return url;
  }
}

// The picture of an audio item (embedded cover or chosen image), if any.
export function artworkOf(item) {
  const artwork = item && item.artwork;
  return artwork && typeof artwork.url === 'string' && artwork.url ? artwork : null;
}

// What a built preview depends on: it is reused while this key is the same.
// The picture of an audio is part of it, so adding, replacing or removing the
// picture rebuilds the preview instead of leaving the old one in the drawer.
export function previewKey(item) {
  const media = (item && item.media) || {};
  const artwork = media.kind === 'audio' ? artworkOf(item) : null;
  return JSON.stringify([media.kind || null, mediaPath(item && item.mediaUrl), artwork ? mediaPath(artwork.url) : null]);
}
