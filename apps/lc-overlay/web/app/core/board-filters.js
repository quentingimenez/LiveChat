// Pure helpers for the board's search, filters and sort. No import: this
// module also loads under Node for its unit tests.

export const SOURCE_LABELS = {
  youtube: 'YouTube',
  tiktok: 'TikTok',
  x: 'X',
  file: 'Fichier',
  direct: 'Lien direct',
};

export const DURATION_LABELS = {
  short: 'Moins de 10 s',
  medium: '10 s à 1 min',
  long: 'Plus d’1 min',
};

export const SORT_LABELS = {
  newest: 'Plus récents',
  oldest: 'Plus anciens',
  title: 'Titre (A → Z)',
  duration: 'Durée (courte → longue)',
};

export const DEFAULT_FILTERS = Object.freeze({
  favorites: false,
  shortcut: false,
  author: '',
  source: '',
  duration: '',
  sort: 'newest',
});

// Where a media comes from, from its source URL.
export function mediaSource(sourceUrl) {
  if (typeof sourceUrl !== 'string' || !sourceUrl) {
    return 'file';
  }
  let url;
  try {
    url = new URL(sourceUrl);
  } catch {
    return 'file';
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return 'file';
  }
  const host = url.hostname.toLowerCase().replace(/^(www|m|mobile|vm|vt)\./, '');
  if (host === 'youtu.be' || host === 'youtube.com' || host.endsWith('.youtube.com')) {
    return 'youtube';
  }
  if (host === 'tiktok.com' || host.endsWith('.tiktok.com')) {
    return 'tiktok';
  }
  if (['x.com', 'twitter.com', 'fxtwitter.com', 'vxtwitter.com', 'fixupx.com'].includes(host)) {
    return 'x';
  }
  return 'direct';
}

// Duration bucket of a media; `null` without a duration (an image).
export function durationBucket(durationSec) {
  const seconds = Number(durationSec);
  if (durationSec === null || durationSec === undefined || !Number.isFinite(seconds)) {
    return null;
  }
  if (seconds < 10) {
    return 'short';
  }
  return seconds <= 60 ? 'medium' : 'long';
}

// Stable identity of the item's author: the member (or Discord account) behind
// `createdByName`. A member renamed after linking Discord keeps one identity, so
// the author filter stays on one entry. Items without it fall back to the name.
export function authorIdOf(item) {
  if (item.createdByAuthorId) {
    return item.createdByAuthorId;
  }
  return item.createdByName ? `name:${item.createdByName}` : null;
}

// Authors that added at least one item, sorted by name.
export function authorsOf(items) {
  const authors = new Map();
  for (const item of items) {
    const id = authorIdOf(item);
    if (id && item.createdByName && !authors.has(id)) {
      authors.set(id, item.createdByName);
    }
  }
  return Array.from(authors, ([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name, 'fr', { sensitivity: 'base' }));
}

// Number of filters that differ from the defaults (sort excluded).
export function activeFilterCount(filters) {
  const f = filters || DEFAULT_FILTERS;
  return [f.favorites, f.shortcut, f.author, f.source, f.duration].filter(Boolean).length;
}

function createdAtMs(item) {
  const value = new Date(item.createdAt).getTime();
  return Number.isFinite(value) ? value : 0;
}

// Items shown on the board: search, kind, filters, then sort.
// `titleOf(item)` gives the displayed title; `shortcutItemIds` is a Set of the
// item ids bound to a keyboard shortcut.
export function selectItems(items, options) {
  const { search = '', kind = 'all', filters = DEFAULT_FILTERS, shortcutItemIds = new Set(), titleOf = (item) => item.title || '' } = options || {};
  const needle = search.trim().toLowerCase();
  const kept = items.filter((item) => {
    const media = item.media || {};
    if (kind !== 'all' && media.kind !== kind) {
      return false;
    }
    if (filters.favorites && !item.favorite) {
      return false;
    }
    if (filters.shortcut && !shortcutItemIds.has(item.id)) {
      return false;
    }
    if (filters.author && authorIdOf(item) !== filters.author) {
      return false;
    }
    if (filters.source && mediaSource(media.sourceUrl) !== filters.source) {
      return false;
    }
    if (filters.duration && durationBucket(media.durationSec) !== filters.duration) {
      return false;
    }
    if (!needle) {
      return true;
    }
    return [titleOf(item), item.message, item.createdByName, media.sourceUrl]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(needle));
  });
  const sort = filters.sort || 'newest';
  const sorted = kept.slice();
  if (sort === 'oldest') {
    sorted.sort((a, b) => createdAtMs(a) - createdAtMs(b));
  } else if (sort === 'title') {
    sorted.sort((a, b) => titleOf(a).localeCompare(titleOf(b), 'fr', { sensitivity: 'base', numeric: true }));
  } else if (sort === 'duration') {
    const length = (item) => {
      const seconds = Number(item.media && item.media.durationSec);
      return Number.isFinite(seconds) && item.media.durationSec !== null ? seconds : Number.POSITIVE_INFINITY;
    };
    sorted.sort((a, b) => length(a) - length(b));
  } else {
    sorted.sort((a, b) => createdAtMs(b) - createdAtMs(a));
  }
  return sorted;
}

// A dropped text that can be imported as a link: the first http(s) URL.
export function droppedLink(text) {
  if (typeof text !== 'string') {
    return null;
  }
  for (const line of text.split(/\r?\n/)) {
    const candidate = line.trim();
    if (!candidate || candidate.startsWith('#')) {
      continue;
    }
    try {
      const url = new URL(candidate);
      if (url.protocol === 'http:' || url.protocol === 'https:') {
        return url.href;
      }
    } catch {
      // Not a URL: keep looking.
    }
  }
  return null;
}
