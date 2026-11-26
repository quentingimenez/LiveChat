// Protects the board message shown when an action cannot reach a guild.
//
// The shell (`undeliverable_reason` in src/net/ws.rs) names why the guild's
// session is down: outdated overlay, connection in progress, revoked device,
// guild cut on this device. Each reason must have its own label: a paired user
// whose overlay was refused as outdated was told « Guilde déconnectée » and
// went looking for a network problem when an update was the fix.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { ERROR_LABELS, describeError } from '../../web/app/core/state.js';

const here = dirname(fileURLToPath(import.meta.url));
const wsSource = readFileSync(resolve(here, '../../src/net/ws.rs'), 'utf8');

// Reasons returned by `undeliverable_reason`, read from the Rust source so a
// reason added there without a label fails here.
function shellReasons() {
  const start = wsSource.indexOf('pub fn undeliverable_reason(');
  assert.notEqual(start, -1, 'undeliverable_reason introuvable dans ws.rs');
  // The function ends at the first closing brace in column 0 (LF or CRLF).
  const end = wsSource.slice(start).search(/\r?\n\}\r?\n/);
  assert.notEqual(end, -1, 'fin de undeliverable_reason introuvable');
  const body = wsSource.slice(start, start + end);
  return [...new Set([...body.matchAll(/"([a-z_]+)"/g)].map((match) => match[1]))];
}

test('the shell reports more than the generic reason', () => {
  const reasons = shellReasons();
  assert.ok(reasons.includes('socket_unavailable'), reasons.join(', '));
  assert.ok(reasons.length > 1, `raisons lues : ${reasons.join(', ')}`);
});

test('every reason the shell can return has a label', () => {
  for (const reason of shellReasons()) {
    assert.equal(typeof ERROR_LABELS[reason], 'string', `libellé manquant pour ${reason}`);
    assert.notEqual(describeError(reason), 'Action impossible.', reason);
  }
});

test('a precise reason never falls back to « Guilde déconnectée »', () => {
  const generic = ERROR_LABELS.socket_unavailable;
  for (const reason of shellReasons().filter((reason) => reason !== 'socket_unavailable')) {
    assert.notEqual(ERROR_LABELS[reason], generic, reason);
  }
});

test('an outdated overlay is told to update', () => {
  assert.match(describeError('overlay_outdated'), /mets.*à jour|mise à jour/i);
});

test('an unknown reason keeps the fallback', () => {
  assert.equal(describeError('not_a_reason'), 'Action impossible.');
  assert.equal(describeError('not_a_reason', 'Repli.'), 'Repli.');
});
