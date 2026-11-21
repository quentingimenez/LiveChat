// Destructive guild actions in the overlay's guild settings: making the bot
// leave the Discord server and deleting the guild. The server side is covered
// by Rust tests; these assertions protect the UI wiring that no browser test
// runs: the `PurgeGuild` gate, the shared strong confirmation, and the exact
// overlay routes (a wrong path would fail only at runtime).

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

const danger = () => read('guild-danger.js');
const view = () => read('views/guild-settings.js');

// The `renderDangerZone` function body, up to the exported entry point.
function dangerZone() {
  const source = view();
  const start = source.indexOf('function renderDangerZone');
  assert.ok(start >= 0, 'renderDangerZone not found');
  const end = source.indexOf('export function renderGuildSettings', start);
  assert.ok(end > start, 'renderDangerZone is not followed by renderGuildSettings');
  return source.slice(start, end);
}

test('the purge dialog calls the overlay purge route with the guild confirmation', () => {
  assert.match(
    danger(),
    /api\('POST', '\/overlay\/admin\/purge', \{ confirmGuildId: guild\.id \}\)/,
  );
  assert.match(danger(), /confirmLabel: 'Supprimer la guilde'/);
  assert.match(danger(), /Membres, overlays, codes, file d’attente, board et réglages/);
});

test('the leave dialog calls the overlay discord-leave route and reports success', () => {
  assert.match(
    danger(),
    /api\('POST', '\/overlay\/admin\/discord-leave', \{ confirmGuildId: guild\.id \}\)/,
  );
  assert.match(danger(), /'Le bot a quitté le serveur Discord\.'/);
});

test('the strong confirmation requires the exact guild name before enabling', () => {
  assert.match(danger(), /const matches = \(\) => field\.value\.trim\(\) === guildName/);
  assert.match(danger(), /confirmButton\.disabled = busy \|\| !matches\(\)/);
  assert.match(danger(), /'Le nom ne correspond pas\.'/);
});

test('a guild without a Discord server gets no leave action', () => {
  // Native guilds always have a server (the guild id is the snowflake);
  // standalone guilds only have one once linked.
  assert.match(
    danger(),
    /export function hasDiscordServer\(guild\) \{\s*return guild\.standalone \? Boolean\(guild\.discordGuildId\) : true;\s*\}/,
  );
  const zone = dangerZone();
  assert.match(zone, /hasDiscordServer\(guild\)/);
  // The leave action needs no guild name; only the purge's typed confirmation
  // does, so a native guild without a `guilds` row can still make the bot leave.
  assert.match(zone, /if \(hasDiscordServer\(guild\)\)/);
  assert.doesNotMatch(zone, /if \(guild\.name && hasDiscordServer\(guild\)\)/);
  assert.match(danger(), /const name = guild\.name \|\| 'cette guilde'/);
});

test('the danger zone is rendered for PurgeGuild after the board settings', () => {
  const source = view();
  assert.match(source, /can\('purgeGuild'\) \? renderDangerZone\(\) : null/);
  const board = source.indexOf('renderBoardSettings()');
  const zone = source.indexOf('renderDangerZone()');
  assert.ok(board >= 0 && zone > board, 'the danger zone comes after « Board de la guilde »');
});

test('the clear-board confirmation still uses the shared typed dialog', () => {
  assert.match(
    view(),
    /openTypedGuildDialog\(\{[\s\S]*?action: \(confirmation\) => api\('POST', '\/overlay\/admin\/media-board\/clear', \{ confirm: confirmation \}\)/,
  );
});
