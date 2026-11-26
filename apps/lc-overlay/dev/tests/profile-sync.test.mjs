// Device profile follower: after a Discord link the board copies the member's
// Discord identity into the local device profile, then keeps it fresh only for
// the remembered Discord account. A manual edit detaches the follow; a new link
// replaces the marker. Guild profiles are server-side and never written here.
//
// The helpers are pure so they run without a DOM. The wiring assertions pin the
// link → adoption path and the fact that only a local data URL reaches
// `settings.profile.avatar`.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  FOLLOWER_KEY,
  PROFILE_SYNC_INTERVAL_MS,
  discordAvatarUrl,
  followsDiscord,
  forgetFollower,
  isLocalAvatar,
  normalizeDiscordId,
  profileMatches,
  profilePatch,
  readFollower,
  rememberFollower,
  shouldSyncProfile,
} from '../../web/app/core/profile-sync.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

function memoryStorage() {
  const values = new Map();
  return {
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

const DISCORD_ACCOUNT = {
  displayName: 'Nova',
  avatar: 'https://cdn.discordapp.com/avatars/123/nova.png?size=128',
  discordUserId: '123',
  discordLinked: true,
  profileSource: 'discord',
};

test('la clé du compte suivi est versionnée', () => {
  assert.equal(FOLLOWER_KEY, 'livechat.profile.follower.v1');
  assert.equal(PROFILE_SYNC_INTERVAL_MS, 6 * 60 * 60 * 1000);
  assert.equal(normalizeDiscordId(' 123 '), '123');
  assert.equal(normalizeDiscordId('abc'), null);
  assert.equal(normalizeDiscordId(123), null);
});

test('le marqueur se mémorise, se lit et s’efface', () => {
  const storage = memoryStorage();
  assert.equal(readFollower(storage), null);
  assert.equal(rememberFollower('123', storage), '123');
  assert.equal(readFollower(storage), '123');
  forgetFollower(storage);
  assert.equal(readFollower(storage), null);
});

test('un nouveau link remplace le compte suivi', () => {
  const storage = memoryStorage();
  rememberFollower('123', storage);
  rememberFollower('456', storage);
  assert.equal(readFollower(storage), '456');
  assert.equal(shouldSyncProfile(DISCORD_ACCOUNT, readFollower(storage)), false);
  assert.equal(
    shouldSyncProfile({ ...DISCORD_ACCOUNT, discordUserId: '456' }, readFollower(storage)),
    true,
  );
});

test('la synchro ne suit que le compte mémorisé qui suit Discord', () => {
  assert.equal(followsDiscord(DISCORD_ACCOUNT), true);
  assert.equal(followsDiscord({ ...DISCORD_ACCOUNT, profileSource: 'custom' }), false);
  assert.equal(followsDiscord({ ...DISCORD_ACCOUNT, discordLinked: false }), false);

  assert.equal(shouldSyncProfile(DISCORD_ACCOUNT, '123'), true);
  assert.equal(shouldSyncProfile(DISCORD_ACCOUNT, '456'), false);
  assert.equal(shouldSyncProfile(DISCORD_ACCOUNT, null), false);
  assert.equal(shouldSyncProfile({ ...DISCORD_ACCOUNT, profileSource: 'custom' }, '123'), false);
  assert.equal(shouldSyncProfile({ ...DISCORD_ACCOUNT, discordUserId: null }, '123'), false);

  // A manual edit clears the marker: the next periodic refresh is a no-op.
  const storage = memoryStorage();
  rememberFollower('123', storage);
  forgetFollower(storage);
  assert.equal(shouldSyncProfile(DISCORD_ACCOUNT, readFollower(storage)), false);
});

test('seule une URL http(s) distante est téléchargée', () => {
  assert.equal(discordAvatarUrl(DISCORD_ACCOUNT), DISCORD_ACCOUNT.avatar);
  assert.equal(discordAvatarUrl({ avatar: '  https://example.test/a.png  ' }), 'https://example.test/a.png');
  assert.equal(discordAvatarUrl({ avatar: 'data:image/png;base64,abc' }), null);
  assert.equal(discordAvatarUrl({ avatar: '/local.png' }), null);
  assert.equal(discordAvatarUrl({}), null);
  assert.equal(isLocalAvatar('data:image/jpeg;base64,abc'), true);
  assert.equal(isLocalAvatar('https://example.test/a.png'), false);
});

test('le patch garde le nom et seulement un avatar local', () => {
  assert.deepEqual(profilePatch(DISCORD_ACCOUNT, 'data:image/jpeg;base64,abc'), {
    displayName: 'Nova',
    avatar: 'data:image/jpeg;base64,abc',
  });
  // Avatar download failed: the existing local avatar must stay untouched.
  assert.deepEqual(profilePatch(DISCORD_ACCOUNT, null), { displayName: 'Nova' });
  assert.deepEqual(profilePatch(DISCORD_ACCOUNT, DISCORD_ACCOUNT.avatar), { displayName: 'Nova' });
  assert.deepEqual(profilePatch({ ...DISCORD_ACCOUNT, displayName: '  Nova  ' }, null), {
    displayName: 'Nova',
  });
  assert.equal(profilePatch({ ...DISCORD_ACCOUNT, displayName: '   ' }, null), null);
  assert.equal(profilePatch(null, 'data:image/jpeg;base64,abc'), null);
});

test('un patch identique n’écrit pas le profil', () => {
  const patch = { displayName: 'Nova', avatar: 'data:image/jpeg;base64,abc' };
  assert.equal(profileMatches(patch, patch), true);
  assert.equal(profileMatches(patch, { displayName: 'Nova' }), true);
  assert.equal(profileMatches(patch, { displayName: 'Autre', avatar: patch.avatar }), false);
  assert.equal(profileMatches(patch, { displayName: 'Nova', avatar: 'data:image/jpeg;base64,def' }), false);
  assert.equal(profileMatches({}, { avatar: patch.avatar }), false);
  assert.equal(profileMatches(null, patch), false);
});

test('le câblage adopte le membre au link et resynchronise au chargement', () => {
  const moduleSource = read('core/profile-sync.js');
  assert.match(moduleSource, /export const FOLLOWER_KEY = 'livechat\.profile\.follower\.v1'/);
  assert.match(moduleSource, /export const PROFILE_SYNC_INTERVAL_MS = 6 \* 60 \* 60 \* 1000/);
  assert.match(moduleSource, /await resizeAvatar\(blob\)/);
  assert.match(moduleSource, /isLocalAvatar\(avatar\)/);
  assert.match(moduleSource, /invoke\('settings_set', \{ payload: \{ profile: patch \} \}\)/);
  assert.match(moduleSource, /invoke\('media_artwork_link', \{ payload: \{ url \} \}\)/);
  assert.match(moduleSource, /state\.profile = \{ \.\.\.state\.profile, \.\.\.patch \}/);
  // A remote avatar never reaches the stored patch.
  assert.doesNotMatch(moduleSource, /avatar:\s*(?:url|member\.avatar|discordAvatarUrl)/);
  // `ipc.js` reads `window` at import time: it must stay a lazy import.
  assert.doesNotMatch(moduleSource, /^import .*ipc\.js/m);

  assert.match(read('views/profile.js'), /syncProfileFromDiscord\(body\.member, \{ force: true \}\)/);
  assert.match(read('data.js'), /syncProfileFromDiscord\(info\.member\)/);
  // The six-hour timer must fetch the member again instead of replaying the
  // cached one, and must not rebuild a settings form that is being edited.
  const main = read('main.js');
  assert.match(main, /initProfileSync\(\{\s*refresh: \(\) => \{/);
  assert.match(main, /return loadGuildInfo\(\{ quiet: true \}\)/);
  assert.doesNotMatch(main, /initProfileSync\(\);/);
});

test('une édition manuelle du profil global détache le suivi', () => {
  const settings = read('views/settings.js');
  assert.match(settings, /import \{ forgetFollower, PROFILE_SYNCED_EVENT \}/);
  const forgetCalls = settings.match(/forgetFollower\(\)/g) || [];
  assert.equal(forgetCalls.length, 3, 'pseudo, changement d’avatar et retrait détachent le suivi');
  assert.match(settings, /PROFILE_SYNCED_EVENT/);
});
