// Broken guild icons must not be attempted again on every rail rebuild.
//
// The rail is rebuilt on each guild event, and inserting a dead <img> again
// replaced the tile's acronym only once the network error came back: the
// acronym ("L’E🔱" for L’Escale) vanished then reappeared, which is the blink
// seen when switching guild. A session cache of broken URLs keeps the acronym
// steady, and the same answer drives the tile's has-icon class so the active
// tile keeps its readable background.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { canLoadIcon, rememberBrokenIcon } from '../../web/app/core/guild-icons.js';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');

test('a guild without an icon never attempts an image', () => {
  assert.equal(canLoadIcon(null), false);
  assert.equal(canLoadIcon(undefined), false);
  assert.equal(canLoadIcon(''), false);
});

test('a fresh icon URL is attempted', () => {
  assert.equal(canLoadIcon('https://cdn.example/fresh.png'), true);
});

test('a broken icon is not attempted again on the next rail rebuild', () => {
  const icon = 'https://cdn.discordapp.com/icons/0/missing.png';
  assert.equal(canLoadIcon(icon), true);
  rememberBrokenIcon(icon);
  assert.equal(canLoadIcon(icon), false);
  // Each rebuild asks again: the answer must stay stable.
  assert.equal(canLoadIcon(icon), false);
});

test('remembering one broken icon leaves the other guilds untouched', () => {
  const broken = 'https://cdn.example/broken.png';
  const other = 'https://cdn.example/other.png';
  rememberBrokenIcon(broken);
  assert.equal(canLoadIcon(broken), false);
  assert.equal(canLoadIcon(other), true);
});

test('the rail asks the cache before building the image and records failures', () => {
  const rail = read('views/rail.js');
  assert.match(rail, /from '\.\.\/core\/guild-icons\.js'/);
  assert.match(rail, /if \(!canLoadIcon\(iconUrl\)\) \{/);
  assert.match(rail, /rememberBrokenIcon\(iconUrl\)/);
  assert.match(rail, /const hasIcon = canLoadIcon\(guild\.membership\.guildIconUrl\)/);
  // The acronym stays the only fallback: no duplicate face code.
  assert.match(rail, /guildAcronym\(label\)/);
});
