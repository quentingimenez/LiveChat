// The board's pause state, read from the view sources: the shared pause lives
// in the media bar (server state), while a guild cut on this device is local
// and must hide both the media bar and the queue so their stop controls cannot
// fail on an empty screen. The views need a DOM, so the assertions read them.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../../web/app/${path}`, import.meta.url), 'utf8');
const queue = read('views/queue.js');
const data = read('data.js');

test('the shared pause is read from the server or device connection state', () => {
  assert.match(queue, /guild\.connection\.paused/);
  assert.match(queue, /media_board_pause/);
  assert.match(queue, /Pause pour tous/);
  assert.match(queue, /Reprendre pour tous/);
});

test('a guild cut on this device hides the media bar and the queue', () => {
  assert.match(queue, /function locallyCut\(\)/);
  assert.match(queue, /!playing \|\| !state\.activeGuildId \|\| locallyCut\(\)/);
  assert.match(queue, /if \(locallyCut\(\)\) \{/);
  assert.match(queue, /Guilde coupée ici/);
  assert.match(queue, /Réactiver ici/);
});

test('a local cut change repaints the board', () => {
  assert.match(data, /livechat:guild-mute-changed/);
  assert.match(queue, /livechat:guild-mute-changed/);
});
