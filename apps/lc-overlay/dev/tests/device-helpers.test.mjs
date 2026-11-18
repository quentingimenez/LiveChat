// Unit tests of the pure device-list helpers (Node's built-in test runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  compareVersions,
  isObsolete,
  lastSeenLabel,
  newestVersion,
  parseVersion,
  platformLabel,
  presenceLabel,
  sessionModeLabel,
} from '../../web/app/core/device-helpers.js';

test('parseVersion reads numeric dotted versions only', () => {
  assert.deepEqual(parseVersion('3.7.0'), [3, 7, 0]);
  assert.deepEqual(parseVersion(' 3.7 '), [3, 7]);
  assert.deepEqual(parseVersion('3.7.0-beta'), [3, 7, 0]);
  for (const value of [null, undefined, '', '   ', 'v3.7', 'abc', 42]) {
    assert.equal(parseVersion(value), null, String(value));
  }
});

test('compareVersions orders dotted versions and treats missing parts as zero', () => {
  assert.equal(compareVersions('3.7.0', '3.7.0'), 0);
  assert.equal(compareVersions('3.7', '3.7.0'), 0);
  assert.equal(compareVersions('3.6.9', '3.7.0'), -1);
  assert.equal(compareVersions('3.7.1', '3.7.0'), 1);
  assert.equal(compareVersions('4', '3.99.99'), 1);
  assert.equal(compareVersions('3.7.0', 'oops'), null);
  assert.equal(compareVersions(null, '3.7.0'), null);
});

test('newestVersion keeps the highest readable version of the list', () => {
  assert.equal(newestVersion([]), null);
  assert.equal(newestVersion([{ overlayVersion: null }, {}]), null);
  assert.equal(
    newestVersion([
      { overlayVersion: '3.5.2' },
      { overlayVersion: '3.7.0' },
      { overlayVersion: 'inconnue' },
      { overlayVersion: '3.6.12' },
    ]),
    '3.7.0',
  );
});

test('isObsolete only flags an older readable version', () => {
  assert.equal(isObsolete('3.5.2', '3.7.0'), true);
  assert.equal(isObsolete('3.7.0', '3.7.0'), false);
  assert.equal(isObsolete('3.8.0', '3.7.0'), false);
  assert.equal(isObsolete(null, '3.7.0'), false);
  assert.equal(isObsolete('3.5.2', null), false);
  assert.equal(isObsolete('inconnue', '3.7.0'), false);
});

test('lastSeenLabel words how long ago a device was seen', () => {
  const now = Date.parse('2026-11-10T12:00:00.000Z');
  const minutes = (count) => new Date(now - count * 60_000).toISOString();
  assert.equal(lastSeenLabel(null, now), 'Jamais vu');
  assert.equal(lastSeenLabel('nope', now), 'Jamais vu');
  assert.equal(lastSeenLabel(minutes(0), now), 'Vu à l’instant');
  assert.equal(lastSeenLabel(new Date(now - 30_000).toISOString(), now), 'Vu à l’instant');
  assert.equal(lastSeenLabel(minutes(3), now), 'Vu il y a 3 min');
  assert.equal(lastSeenLabel(minutes(180), now), 'Vu il y a 3 h');
  assert.equal(lastSeenLabel(minutes(2 * 1440), now), 'Vu il y a 2 j');
});

test('presenceLabel prefers the live state, then pause, then last seen', () => {
  const now = Date.parse('2026-11-10T12:00:00.000Z');
  const lastSeenAt = new Date(now - 3 * 3600_000).toISOString();
  assert.equal(presenceLabel({ connected: true, lastSeenAt }, now), 'En ligne');
  assert.equal(
    presenceLabel({ connected: false, lastSeenAt, guildSettings: { paused: true } }, now),
    'En pause',
  );
  assert.equal(presenceLabel({ connected: false, lastSeenAt }, now), 'Vu il y a 3 h');
  assert.equal(presenceLabel(null, now), '—');
});

test('session mode and platform labels are human readable', () => {
  assert.equal(sessionModeLabel('normal'), 'Contrôle complet');
  assert.equal(sessionModeLabel('invite_read_only'), 'Lecture seule');
  assert.equal(sessionModeLabel('weird'), 'weird');
  assert.equal(sessionModeLabel(null), '—');
  assert.equal(platformLabel('windows'), 'Windows');
  assert.equal(platformLabel('macos'), 'macOS');
  assert.equal(platformLabel('freebsd'), 'freebsd');
  assert.equal(platformLabel(null), '');
});
