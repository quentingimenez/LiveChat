// Follows the Discord profile of the account linked to the board.
//
// A successful `/overlay/profile/discord-link` returns the guild member with
// their Discord display name and avatar. The board copies them into the local
// device profile (`settings.profile`), so the join dialog and the settings page
// show the same identity without storing a remote CDN URL. The guild profiles
// stay server-side: this module never writes them.
//
// The follower marker lives in localStorage and is versioned. A manual edit of
// the device profile clears it; a new link replaces it. Periodic refreshes only
// run while the active guild member follows Discord and their account is the
// remembered one, so a custom guild profile is never overwritten.
//
// Browser globals are only touched inside functions: `node:test` imports this
// module for the pure helpers and must not start a timer or download anything.

import { resizeAvatar } from './avatar.js';
import { state } from './state.js';

export const FOLLOWER_KEY = 'livechat.profile.follower.v1';
export const PROFILE_SYNC_INTERVAL_MS = 6 * 60 * 60 * 1000;
export const PROFILE_SYNCED_EVENT = 'livechat:profile-synced';

function defaultStorage() {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

function storageOf(storage) {
  return storage || defaultStorage();
}

// Discord user ids are numeric snowflakes. Anything else is ignored instead of
// being written to the marker.
export function normalizeDiscordId(value) {
  if (typeof value !== 'string') {
    return null;
  }
  const id = value.trim();
  return /^\d+$/.test(id) ? id : null;
}

export function readFollower(storage) {
  try {
    const store = storageOf(storage);
    return store ? store.getItem(FOLLOWER_KEY) : null;
  } catch {
    return null;
  }
}

export function rememberFollower(discordUserId, storage) {
  const id = normalizeDiscordId(discordUserId);
  if (!id) {
    return null;
  }
  try {
    const store = storageOf(storage);
    if (store) {
      store.setItem(FOLLOWER_KEY, id);
    }
  } catch {
    // A disabled or full localStorage only disables the periodic refresh: the
    // immediate adoption after the link still runs.
  }
  return id;
}

export function forgetFollower(storage) {
  try {
    const store = storageOf(storage);
    if (store) {
      store.removeItem(FOLLOWER_KEY);
    }
  } catch {
    // Nothing to clear.
  }
}

// A member follows Discord only while the server says so: a custom guild
// profile keeps `discordLinked` but stops following.
export function followsDiscord(member) {
  return Boolean(member && member.discordLinked === true && member.profileSource === 'discord');
}

export function shouldSyncProfile(member, followerId) {
  const id = normalizeDiscordId(member && member.discordUserId);
  return Boolean(id && id === normalizeDiscordId(followerId) && followsDiscord(member));
}

// Only a remote avatar is downloaded. A data URL is already local; anything
// else (a relative path, a stale marker) is ignored.
export function discordAvatarUrl(member) {
  const value = member && member.avatar;
  if (typeof value !== 'string') {
    return null;
  }
  const url = value.trim();
  return /^https?:\/\//i.test(url) ? url : null;
}

export function isLocalAvatar(value) {
  return typeof value === 'string' && value.startsWith('data:image/');
}

// The part of the device profile to write. A failed avatar download leaves
// `avatar` out, so the existing local avatar is kept; a display name is never
// cleared by an empty Discord name.
export function profilePatch(member, avatar) {
  if (!member) {
    return null;
  }
  const patch = {};
  const displayName = typeof member.displayName === 'string' ? member.displayName.trim() : '';
  if (displayName) {
    patch.displayName = displayName;
  }
  if (isLocalAvatar(avatar)) {
    patch.avatar = avatar;
  }
  return Object.keys(patch).length > 0 ? patch : null;
}

export function profileMatches(current, patch) {
  if (!current || !patch) {
    return false;
  }
  if (patch.displayName !== undefined && current.displayName !== patch.displayName) {
    return false;
  }
  if (patch.avatar !== undefined && current.avatar !== patch.avatar) {
    return false;
  }
  return patch.displayName !== undefined || patch.avatar !== undefined;
}

async function downloadAvatar(url) {
  if (typeof fetch === 'function') {
    try {
      const response = await fetch(url, { credentials: 'omit', cache: 'no-store' });
      if (response.ok) {
        const blob = await response.blob();
        if (blob && blob.size > 0) {
          return blob;
        }
      }
    } catch {
      // CORS or offline: the shell's own downloader is the fallback.
    }
  }
  try {
    const { invoke } = await import('./ipc.js');
    const linked = await invoke('media_artwork_link', { payload: { url } });
    if (!linked || typeof linked.dataBase64 !== 'string' || !linked.dataBase64) {
      return null;
    }
    const encoded = linked.dataBase64.includes(',')
      ? linked.dataBase64.slice(linked.dataBase64.indexOf(',') + 1)
      : linked.dataBase64;
    const binary = atob(encoded);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return new Blob([bytes], { type: linked.contentType || 'image/jpeg' });
  } catch {
    return null;
  }
}

async function saveProfile(patch) {
  const { invoke } = await import('./ipc.js');
  return invoke('settings_set', { payload: { profile: patch } });
}

function announce(patch) {
  if (typeof document === 'undefined' || typeof CustomEvent !== 'function') {
    return;
  }
  document.dispatchEvent(new CustomEvent(PROFILE_SYNCED_EVENT, { detail: { ...patch } }));
}

async function runSync(member, options) {
  // Imported by tests and by the guild-settings page alike: never touch the
  // network or localStorage outside a browser.
  if (typeof window === 'undefined') {
    return { synced: false, reason: 'not-browser' };
  }
  const force = options.force === true;
  const storage = options.storage || null;
  try {
    if (!member) {
      return { synced: false, reason: 'no-member' };
    }
    const id = normalizeDiscordId(member.discordUserId);
    if (!force && !shouldSyncProfile(member, readFollower(storage))) {
      return { synced: false, reason: 'not-followed' };
    }
    if (force && id) {
      // A new link wins over a previous account: replace the marker before the
      // download, so a failure still leaves the follow in place for the next
      // periodic refresh.
      rememberFollower(id, storage);
    }
    const url = discordAvatarUrl(member);
    let avatar = null;
    if (url) {
      const blob = await downloadAvatar(url);
      avatar = blob ? await resizeAvatar(blob) : null;
    }
    // The user edited the device profile while the avatar was downloading:
    // never overwrite that manual choice.
    if (!force && id && normalizeDiscordId(readFollower(storage)) !== id) {
      return { synced: false, reason: 'detached' };
    }
    const patch = profilePatch(member, avatar);
    if (!patch) {
      return { synced: false, reason: 'empty-profile' };
    }
    const current = options.currentProfile || state.profile || {};
    if (profileMatches(current, patch)) {
      if (id) {
        rememberFollower(id, storage);
      }
      return { synced: true, changed: false, patch };
    }
    const response = await saveProfile(patch);
    if (!response || response.ok === false) {
      return { synced: false, reason: 'save-failed' };
    }
    if (id) {
      rememberFollower(id, storage);
    }
    state.profile = { ...state.profile, ...patch };
    announce(patch);
    return { synced: true, changed: true, patch };
  } catch (error) {
    return { synced: false, reason: 'error', error: String((error && error.message) || error) };
  }
}

let inFlight = null;

// Copies the member's Discord identity into the device profile. A non-forced
// call does nothing unless the member is the remembered follower and follows
// Discord; a forced call (just after a link) adopts the member and replaces the
// marker. Never rejects.
export function syncProfileFromDiscord(member, options = {}) {
  if (inFlight && options.force !== true) {
    return inFlight;
  }
  const job = runSync(member, options).finally(() => {
    if (inFlight === job) {
      inFlight = null;
    }
  });
  inFlight = job;
  return job;
}

let syncTimer = null;

// Periodic refresh only: the guild-info load and the link handler already sync
// at startup and when a guild is opened. Six hours matches the server's own
// Discord member refresh. `refresh` should read the guild member again: a
// replay of the cached one would never see a Discord rename or avatar change.
export function initProfileSync(options = {}) {
  if (syncTimer !== null || typeof setInterval !== 'function') {
    return syncTimer;
  }
  const refresh = typeof options.refresh === 'function' ? options.refresh : null;
  syncTimer = setInterval(() => {
    if (refresh) {
      try {
        const result = refresh();
        if (result && typeof result.catch === 'function') {
          result.catch(() => {});
        }
      } catch {
        // A failed refresh only delays the next update.
      }
      return;
    }
    const member = state.info && state.info.member;
    if (!member) {
      return;
    }
    syncProfileFromDiscord(member).catch(() => {});
  }, PROFILE_SYNC_INTERVAL_MS);
  return syncTimer;
}
