// Per-guild snapshot cache, in memory only.
//
// Switching guild used to blank the state and wait for the server: the rail
// repainted, then the header and the board stayed on the previous guild until
// `/overlay/guild` landed. The cache lets `applyGuilds` paint the last known
// snapshot immediately and revalidate it in the background.

export const CACHE_TTL = Object.freeze({
  info: 60_000,
  items: 30_000,
  queue: 10_000,
  bindings: 300_000,
  history: 60_000,
});

export const MAX_GUILDS = 8;

// guildId -> { at: { [key]: timestamp }, info, items, queue, bindings, history, usedAt }
const cache = new Map();

function entryFor(guildId, now) {
  let entry = cache.get(guildId);
  if (!entry) {
    entry = { at: {}, usedAt: now };
    cache.set(guildId, entry);
  }
  entry.usedAt = now;
  return entry;
}

// Returns `null` when the guild has no value for this key, otherwise
// `{ value, at, age, fresh }`. Reading refreshes the LRU position.
export function readGuildCache(guildId, key, now = Date.now()) {
  if (!guildId) {
    return null;
  }
  const entry = cache.get(guildId);
  if (!entry || !Object.hasOwn(entry.at, key)) {
    return null;
  }
  entry.usedAt = now;
  const at = entry.at[key];
  const ttl = CACHE_TTL[key] || 0;
  return { value: entry[key], at, age: now - at, fresh: ttl > 0 && now - at <= ttl };
}

export function writeGuildCache(guildId, key, value, now = Date.now()) {
  if (!guildId || !Object.hasOwn(CACHE_TTL, key)) {
    return;
  }
  const entry = entryFor(guildId, now);
  entry[key] = value;
  entry.at[key] = now;
  pruneToLimit();
}

// Drops one key, or the whole guild when `key` is omitted.
export function invalidateGuildCache(guildId, key) {
  if (!guildId) {
    return;
  }
  if (!key) {
    cache.delete(guildId);
    return;
  }
  const entry = cache.get(guildId);
  if (!entry) {
    return;
  }
  delete entry[key];
  delete entry.at[key];
  if (Object.keys(entry.at).length === 0) {
    cache.delete(guildId);
  }
}

// Forgets guilds the shell no longer knows (left, kicked, removed).
export function pruneGuildCache(validIds) {
  const keep = new Set(validIds || []);
  for (const guildId of cache.keys()) {
    if (!keep.has(guildId)) {
      cache.delete(guildId);
    }
  }
}

function pruneToLimit() {
  while (cache.size > MAX_GUILDS) {
    let oldestId = null;
    let oldestUse = Infinity;
    for (const [guildId, entry] of cache) {
      if (entry.usedAt < oldestUse) {
        oldestUse = entry.usedAt;
        oldestId = guildId;
      }
    }
    if (oldestId === null) {
      return;
    }
    cache.delete(oldestId);
  }
}

export function clearGuildCache() {
  cache.clear();
}

export function guildCacheSize() {
  return cache.size;
}
