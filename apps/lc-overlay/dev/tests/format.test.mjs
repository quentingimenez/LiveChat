// Unit tests of the pure formatting helpers (Node's built-in test runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  firstGrapheme,
  initials,
  mediaRefreshDelay,
  removalMessage,
} from '../../web/app/core/format.js';

test('initials keeps the first graphemes of significant words', () => {
  const cases = [
    ['L’Escale 🔱', 'LE'],
    ['Les Sardines', 'LS'],
    ['gaming', 'GA'],
    ['Été', 'ÉT'],
    ['Jean-Pierre', 'JP'],
    ["l'équipe", 'LÉ'],
    ['R2D2', 'R2'],
    ['🔱', '🔱'],
    ['👨‍👩‍👧 Famille', 'FA'],
    ['𝐒𝐮𝐩𝐞𝐫𝐊𝐧𝐚𝐜𝐤𝐲', 'SU'],
    ['e\u0301cole', 'ÉC'],
    ['', '?'],
    [null, '?'],
    [undefined, '?'],
    ['x', 'X'],
  ];
  for (const [input, expected] of cases) {
    assert.equal(initials(input), expected, `initials(${JSON.stringify(input) ?? 'undefined'})`);
  }
});

test('firstGrapheme keeps a whole grapheme for avatar letters', () => {
  const cases = [
    ['𝐒𝐮𝐩𝐞𝐫𝐊𝐧𝐚𝐜𝐤𝐲', 'S'],
    ['L’Escale 🔱', 'L'],
    ['🔱', '🔱'],
    ['👨‍👩‍👧', '👨‍👩‍👧'],
    ['', '?'],
  ];
  for (const [input, expected] of cases) {
    assert.equal(firstGrapheme(initials(input)), expected, `firstGrapheme(initials(${JSON.stringify(input)})`);
  }
});

test('mediaRefreshDelay reloads five minutes before the earliest signed URL expiry', () => {
  const now = 1_700_000_000_000;
  const minute = 60_000;
  assert.equal(mediaRefreshDelay([], now), null);
  assert.equal(mediaRefreshDelay([{ id: 'a' }], now), null);
  assert.equal(mediaRefreshDelay([{ mediaUrlExpiresAt: 'nope' }], now), null);
  assert.equal(mediaRefreshDelay([{ mediaUrlExpiresAt: now + 10 * minute }], now), 5 * minute);
  assert.equal(mediaRefreshDelay([{ mediaUrlExpiresAt: now + 60 * minute }, { mediaUrlExpiresAt: now + 6 * minute }], now), minute);
  assert.equal(mediaRefreshDelay([{ mediaUrlExpiresAt: now + 2 * minute }], now), 0);
  assert.equal(mediaRefreshDelay([{ mediaUrlExpiresAt: now + 30 * minute }, { mediaUrlExpiresAt: 0 }], now), 25 * minute);
});

test('removalMessage names the guild for every removal reason', () => {
  assert.equal(
    removalMessage({ guildName: 'Les Sardines', reason: 'guild_deleted' }),
    '« Les Sardines » a été supprimée par un admin et retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ guildName: 'Les Sardines', reason: 'member_removed' }),
    'Un admin t’a retiré de « Les Sardines ». La guilde est retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ guildName: 'Les Sardines', reason: 'revoked' }),
    'Cet appareil a été révoqué dans « Les Sardines ». La guilde est retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ guildName: 'Les Sardines', reason: 'invalid_token' }),
    '« Les Sardines » ne reconnaît plus cet appareil. La guilde est retirée de LiveChat.',
  );
  // Unknown reasons use the invalid-token wording.
  assert.equal(
    removalMessage({ guildName: 'Les Sardines', reason: 'something_else' }),
    '« Les Sardines » ne reconnaît plus cet appareil. La guilde est retirée de LiveChat.',
  );
});

test('removalMessage falls back when the guild name is missing', () => {
  assert.equal(
    removalMessage({ reason: 'guild_deleted' }),
    'Une guilde a été supprimée par un admin et retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ guildName: '  ', reason: 'member_removed' }),
    'Un admin t’a retiré de une guilde. La guilde est retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ reason: 'revoked' }),
    'Cet appareil a été révoqué dans une guilde. La guilde est retirée de LiveChat.',
  );
  assert.equal(
    removalMessage({ reason: 'invalid_token' }),
    'Une guilde ne reconnaît plus cet appareil. La guilde est retirée de LiveChat.',
  );
  assert.equal(
    removalMessage(),
    'Une guilde ne reconnaît plus cet appareil. La guilde est retirée de LiveChat.',
  );
});
