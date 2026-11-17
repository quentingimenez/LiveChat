import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  currentUpdateStatus,
  setUpdateStatus,
  subscribeUpdateStatus,
  updateNotice,
} from '../../web/app/core/update-status.js';

const status = (state, extra = {}) => ({
  state,
  currentVersion: '3.1.0',
  availableVersion: null,
  message: '…',
  ...extra,
});

test('no banner outside ready and downloading', () => {
  assert.equal(updateNotice(null), null);
  assert.equal(updateNotice(undefined), null);
  assert.equal(updateNotice({}), null);
  assert.equal(updateNotice('ready'), null);
  for (const state of ['idle', 'checking', 'up_to_date', 'error']) {
    assert.equal(updateNotice(status(state)), null, state);
  }
});

test('a ready update offers to install its version', () => {
  assert.deepEqual(updateNotice(status('ready', { availableVersion: '3.2.0' })), {
    kind: 'ready',
    version: '3.2.0',
    message: 'LiveChat 3.2.0 est prête à être installée.',
  });
});

test('a ready status without a version never shows a broken banner', () => {
  assert.equal(updateNotice(status('ready')), null);
  assert.equal(updateNotice(status('ready', { availableVersion: '' })), null);
});

test('a downloading update shows a progress line only when the version is known', () => {
  assert.deepEqual(updateNotice(status('downloading', { availableVersion: '3.2.0' })), {
    kind: 'downloading',
    version: '3.2.0',
    message: 'Téléchargement de LiveChat 3.2.0…',
  });
  assert.equal(updateNotice(status('downloading')), null);
});

test('a postponed version hides the ready banner, a newer one shows it again', () => {
  const ready = status('ready', { availableVersion: '3.2.0' });
  assert.equal(updateNotice(ready, '3.2.0'), null);
  const newer = updateNotice(status('ready', { availableVersion: '3.3.0' }), '3.2.0');
  assert.deepEqual(newer, {
    kind: 'ready',
    version: '3.3.0',
    message: 'LiveChat 3.3.0 est prête à être installée.',
  });
});

test('a postponed version does not hide the downloading banner', () => {
  // Postponing only applies to the install prompt: the download itself stays
  // informative, and the banner disappears once it is done.
  const downloading = status('downloading', { availableVersion: '3.2.0' });
  assert.equal(updateNotice(downloading, '3.2.0').kind, 'downloading');
});

test('the shared store notifies its subscribers and lets them go', () => {
  const seen = [];
  const unsubscribe = subscribeUpdateStatus((value) => seen.push(value));
  setUpdateStatus(status('ready', { availableVersion: '3.2.0' }));
  assert.equal(currentUpdateStatus().state, 'ready');
  setUpdateStatus(null);
  assert.equal(currentUpdateStatus(), null);
  unsubscribe();
  setUpdateStatus(status('downloading', { availableVersion: '3.2.0' }));
  // The unsubscribed listener heard the two first changes only.
  assert.equal(seen.length, 2);
  assert.equal(seen[0].state, 'ready');
  assert.equal(seen[1], null);
  // Leave the store clean for the other tests.
  setUpdateStatus(null);
});
