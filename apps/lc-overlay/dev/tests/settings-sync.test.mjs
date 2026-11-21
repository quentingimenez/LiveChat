// Guild settings can be edited from several boards at once. The server now
// versions every writable setting, refuses a stale PATCH instead of silently
// overwriting it, and wakes the other boards after every change. These
// assertions pin both halves of that contract: the server refuses stale writes,
// and a board that is editing a field shows a banner rather than replacing the
// form under the cursor.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { guildLabel, state } from '../../web/app/core/state.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const readServer = (path) =>
  readFileSync(new URL(`../../../lc-server/src/http/overlay/${path}`, import.meta.url), 'utf8');
const readShell = (path) => readFileSync(new URL(`../../src/${path}`, import.meta.url), 'utf8');

const guild = readServer('guild.rs');
const admin = readServer('admin.rs');
const events = readShell('events.rs');
const ws = readShell('net/ws.rs');
const dom = read('core/dom.js');
const settings = read('views/settings.js');
const data = read('data.js');
const main = read('main.js');
const adminView = read('views/admin.js');
const header = read('views/header.js');

test('GET /overlay/guild publishes a version of every writable setting', () => {
  assert.match(guild, /pub\(super\) fn settings_version/);
  assert.match(guild, /"settingsVersion": settings_version/);
});

test('a stale settings PATCH is refused with settings_conflict', () => {
  assert.match(admin, /expectedSettingsVersion/);
  assert.match(admin, /StatusCode::CONFLICT, "settings_conflict"/);
});

test('every settings write wakes the other boards', () => {
  assert.match(admin, /state\.admin_hub\.broadcast_settings\(guild_id\)\.await;/);
  assert.ok(!admin.includes('playback_changed'), 'the old playback-only gate is gone');
});

test('the shell forwards a guild-settings event to the board window', () => {
  assert.match(events, /EVENT_GUILD_SETTINGS_CHANGED: &str = "overlay:\/\/guild-settings-changed"/);
  assert.match(events, /EventTarget::labeled\(BOARD_LABEL\),\s*EVENT_GUILD_SETTINGS_CHANGED/);
  assert.match(ws, /emit_guild_settings_changed\(&self\.app, &self\.guild_id\)/);
});

test('an active settings field is never replaced by a background refresh', () => {
  assert.match(dom, /export function settingsControlActive\(\)/);
  assert.match(data, /settingsControlActive\(\)/);
  assert.ok(
    data.indexOf('settingsControlActive()') < data.indexOf('state.info = info;'),
    'the guard runs before the form state is replaced',
  );
});

test('the board shows a reload banner and saves with the version it read', () => {
  assert.match(main, /overlay:\/\/guild-settings-changed/);
  assert.match(main, /loadGuildInfo\(\{ quiet: true, force: true \}\)/);
  assert.match(data, /options\.force/);
  assert.match(settings, /export function showSettingsRemoteUpdate\(\)/);
  assert.match(settings, /modifiés sur un autre appareil/);
  assert.match(header, /loadGuildInfo\(\{ quiet: true(?:, force: true)? \}\)/);
  assert.match(adminView, /expectedSettingsVersion/);
  assert.match(adminView, /export async function saveGuildSettings/);
  assert.match(adminView, /error\.code === 'settings_conflict'/);
});

test('the header name follows the live guild info, not the invite-time cache', () => {
  const previous = { activeGuildId: state.activeGuildId, info: state.info };
  state.activeGuildId = 'guild-live';
  state.info = { guild: { name: 'Nouveau nom' } };
  assert.equal(
    guildLabel({ membership: { guildId: 'guild-live', guildName: 'Ancien nom' } }),
    'Nouveau nom',
  );

  state.info = null;
  assert.equal(
    guildLabel({ membership: { guildId: 'guild-live', guildName: 'Ancien nom' } }),
    'Ancien nom',
    'the cached name still paints before /overlay/guild answers',
  );

  state.activeGuildId = previous.activeGuildId;
  state.info = previous.info;
});
