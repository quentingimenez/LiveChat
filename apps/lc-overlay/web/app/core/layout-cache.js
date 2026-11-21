// Persistent, tiny description of the last board layout seen per guild.
//
// The first paint of a guild cannot know the real media sizes before the
// server answers. This cache keeps only the shape of each media (kind, width,
// height), never ids, titles or URLs, so the skeleton of a guild already
// visited can reuse the sizes of its last visit instead of generic cards.
//
// Storage is injected (localStorage in the app, a fake Map in the tests); a
// missing or broken storage simply disables the cache.

export const LAYOUT_STORAGE_KEY = 'livechat.board-layout.v1';
export const LAYOUT_VERSION = 1;
export const MAX_LAYOUT_GUILDS = 8;
// The board asks for 200 items; 60 shapes are enough to fill any viewport and
// keep the persisted payload small.
export const MAX_LAYOUT_ITEMS = 60;

function defaultStorage() {
  try {
    return globalThis.localStorage || null;
  } catch {
    return null;
  }
}

function emptyStore() {
  return { version: LAYOUT_VERSION, guilds: {} };
}

function readStore(storage) {
  if (!storage) {
    return emptyStore();
  }
  try {
    const raw = storage.getItem(LAYOUT_STORAGE_KEY);
    if (!raw) {
      return emptyStore();
    }
    const parsed = JSON.parse(raw);
    if (
      !parsed ||
      parsed.version !== LAYOUT_VERSION ||
      !parsed.guilds ||
      typeof parsed.guilds !== 'object'
    ) {
      return emptyStore();
    }
    return parsed;
  } catch {
    return emptyStore();
  }
}

function writeStore(storage, store) {
  if (!storage) {
    return false;
  }
  try {
    storage.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(store));
    return true;
  } catch {
    // Quota exceeded, private mode or a disabled storage: the skeleton falls
    // back to its generic shapes, never breaks the board.
    return false;
  }
}

function positive(value) {
  const number = Number(value);
  return Number.isFinite(number) && number > 0 ? Math.round(number) : null;
}

function normalizeItem(item) {
  if (!item || typeof item !== 'object') {
    return null;
  }
  // A board item carries its media under `media` and may show custom artwork
  // instead (the card's ratio follows the artwork in that case).
  const media = item.media && typeof item.media === 'object' ? item.media : item;
  const artwork = item.artwork && typeof item.artwork === 'object' ? item.artwork : null;
  const artworkWidth = artwork && positive(artwork.width);
  const artworkHeight = artwork && positive(artwork.height);
  const useArtwork = Boolean(artworkWidth && artworkHeight);
  const source = useArtwork ? artwork : media;
  const kind = useArtwork
    ? 'image'
    : source.kind === 'audio' || source.kind === 'image' || source.kind === 'video'
      ? source.kind
      : null;
  return {
    kind,
    width: positive(source.width),
    height: positive(source.height),
  };
}

function trimStore(store) {
  const ids = Object.keys(store.guilds).sort(
    (a, b) => (Number(store.guilds[b].at) || 0) - (Number(store.guilds[a].at) || 0),
  );
  for (const id of ids.slice(MAX_LAYOUT_GUILDS)) {
    delete store.guilds[id];
  }
}

// Returns `{ items, at }` for a guild that has a usable layout, else null.
export function readLayoutCache(guildId, storage = defaultStorage()) {
  if (!guildId) {
    return null;
  }
  const store = readStore(storage);
  const entry = store.guilds[guildId];
  if (!entry || !Array.isArray(entry.items) || entry.items.length === 0) {
    return null;
  }
  const items = entry.items.map(normalizeItem).filter(Boolean);
  if (items.length === 0) {
    return null;
  }
  return { items, at: Number(entry.at) || 0 };
}

// Stores the last layout of `guildId`; keeps the most recently visited guilds.
export function writeLayoutCache(guildId, items, storage = defaultStorage(), now = Date.now()) {
  if (!guildId || !Array.isArray(items)) {
    return false;
  }
  const store = readStore(storage);
  const normalized = items
    .slice(0, MAX_LAYOUT_ITEMS)
    .map(normalizeItem)
    .filter(Boolean);
  store.guilds[guildId] = { at: now, items: normalized };
  trimStore(store);
  return writeStore(storage, store);
}

// Drops layouts of guilds that are no longer known by the shell.
export function pruneLayoutCache(validIds, storage = defaultStorage()) {
  const valid = new Set((validIds || []).filter(Boolean));
  const store = readStore(storage);
  let changed = false;
  for (const id of Object.keys(store.guilds)) {
    if (!valid.has(id)) {
      delete store.guilds[id];
      changed = true;
    }
  }
  if (changed) {
    writeStore(storage, store);
  }
  return changed;
}

export function clearLayoutCache(storage = defaultStorage()) {
  if (!storage) {
    return;
  }
  try {
    storage.removeItem(LAYOUT_STORAGE_KEY);
  } catch {
    // Nothing to do: the cache is optional.
  }
}

export function layoutCacheSize(storage = defaultStorage()) {
  return Object.keys(readStore(storage).guilds).length;
}
