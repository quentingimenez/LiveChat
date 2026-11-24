// Shared updater status store (core/update-status.js): Paramètres → À propos is
// now the only surface that shows updater state, so this file protects the
// store contract it relies on (current value, clearing, subscriptions). The
// board banner and its `updateNotice` helper were removed on purpose.
//
// The updater's own policy is tested in Rust (src/updater.rs).

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  currentUpdateStatus,
  setUpdateStatus,
  subscribeUpdateStatus,
} from '../../web/app/core/update-status.js';

const status = (state, extra = {}) => ({
  state,
  currentVersion: '3.1.0',
  availableVersion: null,
  message: '',
  ...extra,
});

test('a fresh store has no status', () => {
  assert.equal(currentUpdateStatus(), null);
});

test('a status is recorded as-is and anything else clears it', () => {
  const ready = status('ready', { availableVersion: '3.2.0' });
  setUpdateStatus(ready);
  assert.deepEqual(currentUpdateStatus(), ready);

  for (const value of [null, undefined, 'ready', 42]) {
    setUpdateStatus(value);
    assert.equal(currentUpdateStatus(), null);
  }
});

test('subscribers see every change until they unsubscribe', () => {
  setUpdateStatus(null);
  const seen = [];
  const unsubscribe = subscribeUpdateStatus((value) => seen.push(value));
  const ready = status('ready', { availableVersion: '3.2.0' });

  setUpdateStatus(ready);
  setUpdateStatus(null);
  assert.deepEqual(seen, [ready, null]);

  unsubscribe();
  setUpdateStatus(status('checking'));
  assert.deepEqual(seen, [ready, null]);

  setUpdateStatus(null);
});
