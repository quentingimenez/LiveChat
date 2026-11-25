// Debounced media prefetch for the board's cards.
//
// The board's items already carry every piece of the detail window's
// information; what makes the window feel slow in production is the media
// itself: the first frame of a video, or a large picture, only starts loading
// when the window opens. This module starts that load as soon as the pointer
// rests on a card (after a short debounce, so sweeping across the grid loads
// nothing) and hands the already-loading element to the hover preview and to
// the detail window, which then opens on bytes the browser has.
//
// One prefetch at a time is pending; a newer card replaces it. Completed
// entries are kept keyed by their media URL, up to `limit`, so moving back and
// forth between a few cards stays instant; the oldest is released when the
// limit is exceeded. A signed URL close to its expiry is never prefetched: the
// detail window would otherwise open on a URL that expires while it loads.

export const PREFETCH_DELAY_MS = 120;
export const PREFETCH_LIMIT = 2;
// A signed URL must have at least this much life left to be worth warming.
export const PREFETCH_EXPIRY_MARGIN_MS = 60 * 1000;

const KINDS = new Set(['video', 'audio', 'image']);

function normalizeKind(kind) {
  return KINDS.has(kind) ? kind : null;
}

function expiredAt(expiresAt, now, marginMs) {
  return Number.isFinite(expiresAt) && expiresAt > 0 && expiresAt - now <= marginMs;
}

function releaseElement(element) {
  if (!element) {
    return;
  }
  if (typeof element.pause === 'function') {
    try {
      element.pause();
    } catch {
      // A detached element may already be torn down.
    }
  }
  if (typeof element.removeAttribute === 'function') {
    element.removeAttribute('src');
  }
  // An <img> with no src must not be `load()`ed: only media elements have a
  // load algorithm to abort.
  if (element.tagName !== 'IMG' && typeof element.load === 'function') {
    try {
      element.load();
    } catch {
      // Ignore a media element the platform already released.
    }
  }
}

// options.document/options.clock are injectable so the timing rules stay
// testable without a DOM. They default to the browser's, read at call time.
export function createMediaPrefetcher(options) {
  const opts = options || {};
  const doc = opts.document || (typeof globalThis !== 'undefined' ? globalThis.document : null);
  const clock = opts.clock || {
    now: () => Date.now(),
    setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
    clearTimeout: (id) => globalThis.clearTimeout(id),
  };
  const delayMs = Number.isFinite(opts.delayMs) ? opts.delayMs : PREFETCH_DELAY_MS;
  const limit = Number.isFinite(opts.limit) && opts.limit > 0 ? opts.limit : PREFETCH_LIMIT;
  // url -> { id, kind, element, expiresAt, ready }
  const entries = new Map();
  // { id, url, timer } — the card waiting out the debounce.
  let pending = null;

  function usable(kind, url) {
    return Boolean(kind && url && doc && typeof doc.createElement === 'function');
  }

  function cancelPending() {
    if (!pending) {
      return;
    }
    clock.clearTimeout(pending.timer);
    pending = null;
  }

  function markReady(entry) {
    entry.ready = true;
  }

  function drop(url) {
    const entry = entries.get(url);
    if (!entry) {
      return;
    }
    entries.delete(url);
    releaseElement(entry.element);
  }

  function prune() {
    while (entries.size > limit) {
      const oldest = entries.keys().next().value;
      drop(oldest);
    }
  }

  function load(id, kind, url, expiresAt) {
    if (entries.has(url) || expiredAt(expiresAt, clock.now(), PREFETCH_EXPIRY_MARGIN_MS)) {
      return;
    }
    if (!usable(kind, url)) {
      return;
    }

    const element = doc.createElement(kind === 'image' ? 'img' : kind);
    element.className = 'hp-prefetch';
    const entry = { id, kind, element, expiresAt, ready: false };

    // Listen before the source is set: a small picture may load synchronously
    // from the cache.
    const ready = () => markReady(entry);
    const failed = () => drop(url);
    if (kind === 'image') {
      element.alt = '';
      element.draggable = false;
      element.decoding = 'async';
      element.addEventListener('load', ready);
      element.addEventListener('error', failed);
    } else {
      element.preload = 'auto';
      element.muted = true;
      element.defaultMuted = true;
      element.setAttribute('muted', '');
      element.setAttribute('aria-hidden', 'true');
      if (kind === 'video') {
        element.playsInline = true;
        element.setAttribute('playsinline', '');
      }
      element.addEventListener('loadeddata', ready);
      element.addEventListener('canplay', ready);
      element.addEventListener('error', failed);
    }
    entries.set(url, entry);
    prune();
    element.src = url;
  }

  function schedule(id, desc) {
    if (!id || !desc || typeof desc !== 'object') {
      return;
    }
    const url = typeof desc.url === 'string' ? desc.url : '';
    const kind = normalizeKind(desc.kind);
    if (!usable(kind, url) || expiredAt(desc.expiresAt, clock.now(), PREFETCH_EXPIRY_MARGIN_MS)) {
      return;
    }
    // Already warm: a hover on the same card must not restart the delay.
    if (entries.has(url)) {
      cancelPending();
      return;
    }
    if (pending && pending.id === id && pending.url === url) {
      return;
    }
    cancelPending();
    pending = {
      id,
      url,
      timer: clock.setTimeout(() => {
        const request = pending;
        pending = null;
        if (request) {
          load(request.id, kind, url, desc.expiresAt);
        }
      }, delayMs),
    };
  }

  function cancel(id) {
    if (pending && pending.id === id) {
      cancelPending();
    }
  }

  function take(desc) {
    if (!desc || typeof desc !== 'object') {
      return null;
    }
    const url = typeof desc.url === 'string' ? desc.url : '';
    const kind = normalizeKind(desc.kind);
    const entry = entries.get(url);
    if (!entry) {
      return null;
    }
    entries.delete(url);
    if (entry.kind !== kind || expiredAt(desc.expiresAt, clock.now(), PREFETCH_EXPIRY_MARGIN_MS)) {
      releaseElement(entry.element);
      return null;
    }
    entry.element.classList.remove('hp-prefetch');
    return { kind: entry.kind, element: entry.element, ready: entry.ready };
  }

  function clear() {
    cancelPending();
    for (const url of [...entries.keys()]) {
      drop(url);
    }
  }

  return {
    schedule,
    cancel,
    take,
    clear,
    size: () => entries.size,
    pendingCard: () => (pending ? pending.id : null),
  };
}
