// Unit tests of the pure formatting helpers (Node's built-in test runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  firstGrapheme,
  formatBytes,
  guildAcronym,
  guildAcronymFontSize,
  initials,
  mediaRefreshDelay,
  parseDisplayDuration,
  relativeDateLabel,
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

test('guildAcronym keeps one letter per word in the typed case, like Discord', () => {
  const cases = [
    // The reported case: Discord shows "Sdb", not the initials "SD".
    ['Serveur de babz', 'Sdb'],
    ['Les Sardines', 'LS'],
    ['GG Squad', 'GS'],
    // Discord does not uppercase: "gaming" gives one lowercase letter.
    ['gaming', 'g'],
    // The English possessive is dropped ("Bob's server" has two words).
    ["Bob's server", 'Bs'],
    // Only the ASCII "'s " rule, as in Discord: the typographic apostrophe keeps
    // its word ("Bob’s server" → "B’ss", not "Bs").
    ['Bob’s server', 'B’ss'],
    // Punctuation and emoji between the words are kept as Discord does.
    ['Jean-Pierre', 'J-P'],
    ['L’Escale 🔱', 'L’E🔱'],
    ['R2D2', 'R'],
    // Unicode letters count as word characters (Discord only knows [A-Za-z0-9_]).
    ['Été', 'É'],
    ['𝐒𝐮𝐩𝐞𝐫', 'S'],
    // No length limit: one letter per word, whatever the word count.
    ['Le serveur des amis du soir', 'Lsdads'],
    ['', ''],
    [null, ''],
    [undefined, ''],
  ];
  for (const [input, expected] of cases) {
    assert.equal(guildAcronym(input), expected, `guildAcronym(${JSON.stringify(input) ?? 'undefined'})`);
  }
});

test('guildAcronymFontSize shrinks a long acronym like Discord', () => {
  // Four letters keep the rail's usual size; then Discord's steps apply.
  for (const short of ['S', 'SD', 'Sdb', 'LSDA', '']) {
    assert.equal(guildAcronymFontSize(short), 13, short);
  }
  assert.equal(guildAcronymFontSize('LSDAD'), 11);
  assert.equal(guildAcronymFontSize('LSDADS'), 10);
  assert.equal(guildAcronymFontSize('LSDADSL'), 8);
  assert.equal(guildAcronymFontSize('LSDADSLD'), 8);
  // A grapheme counts once, whatever its number of code units.
  assert.equal(guildAcronymFontSize('🔱🔱🔱🔱🔱'), 11);
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

test('relativeDateLabel names the last week, then the short date', () => {
  const now = new Date(2026, 6, 14, 18, 0, 0);
  const at = (year, month, day, hour = 12) => new Date(year, month - 1, day, hour).toISOString();
  assert.equal(relativeDateLabel(at(2026, 7, 14), now), "aujourd'hui");
  // A future date (clock skew) never says « il y a -1 j ».
  assert.equal(relativeDateLabel(at(2026, 7, 15), now), "aujourd'hui");
  assert.equal(relativeDateLabel(at(2026, 7, 13), now), 'hier');
  assert.equal(relativeDateLabel(at(2026, 7, 12), now), 'il y a 2 j');
  assert.equal(relativeDateLabel(at(2026, 7, 8), now), 'il y a 6 j');
  assert.equal(relativeDateLabel(at(2026, 7, 7), now), '7 juil.');
  assert.equal(relativeDateLabel(at(2026, 1, 3), now), '3 janv.');
  assert.equal(relativeDateLabel('', now), null);
  assert.equal(relativeDateLabel('not a date', now), null);
  // No date at all: nothing to show, not the Unix epoch.
  assert.equal(relativeDateLabel(null, now), null);
  assert.equal(relativeDateLabel(undefined, now), null);
});

test('relativeDateLabel names every month in short French', () => {
  const now = new Date(2027, 0, 31, 12);
  const labels = Array.from({ length: 12 }, (_, month) => relativeDateLabel(new Date(2026, month, 2, 12).toISOString(), now));
  assert.deepEqual(labels, ['2 janv.', '2 févr.', '2 mars', '2 avr.', '2 mai', '2 juin', '2 juil.', '2 août', '2 sept.', '2 oct.', '2 nov.', '2 déc.']);
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
    removalMessage({ guildName: 'Les Sardines', reason: 'member_banned' }),
    'Un admin t’a banni de « Les Sardines ». La guilde est retirée de LiveChat.',
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

test('parseDisplayDuration reads whole seconds from 1 to 600, empty as default', () => {
  assert.deepEqual(parseDisplayDuration(''), { ok: true, value: null });
  assert.deepEqual(parseDisplayDuration('  '), { ok: true, value: null });
  assert.deepEqual(parseDisplayDuration('8'), { ok: true, value: 8 });
  assert.deepEqual(parseDisplayDuration(' 600 '), { ok: true, value: 600 });
  for (const raw of ['0', '601', '8.5', 'abc', '-3', '1e2']) {
    assert.deepEqual(parseDisplayDuration(raw), { ok: false }, raw);
  }
});

test('formatBytes shows Go and Mo with French decimals', () => {
  assert.equal(formatBytes(0), '0 Ko');
  assert.equal(formatBytes(512 * 1024), '512 Ko');
  assert.equal(formatBytes(1024 * 1024), '1 Mo');
  assert.equal(formatBytes(1.5 * 1024 * 1024), '1,5 Mo');
  assert.equal(formatBytes(5 * 1024 * 1024 * 1024), '5 Go');
  assert.equal(formatBytes(1.2 * 1024 * 1024 * 1024), '1,2 Go');
  assert.equal(formatBytes(null), null);
  assert.equal(formatBytes(-1), null);
});
