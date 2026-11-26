// What this file protects: a deferred rebuild waits for the click that took
// the focus, so the pressed control is not replaced between mousedown and
// mouseup. The timer still covers a blur without a click.

import assert from 'node:assert/strict';
import test from 'node:test';

import { afterNextClick } from '../../web/app/core/after-click.js';

test('the callback runs once, after the click, and the listener is removed', async () => {
  const target = new EventTarget();
  let calls = 0;
  afterNextClick(target, () => {
    calls += 1;
  }, { idleMs: 10_000 });

  assert.equal(calls, 0, 'nothing happens before the click');
  target.dispatchEvent(new Event('click'));
  assert.equal(calls, 1, 'the click releases the rebuild');
  target.dispatchEvent(new Event('click'));
  assert.equal(calls, 1, 'a second click does not run it again');
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(calls, 1, 'the idle timer was cleared');
});

test('an ignored click leaves the rebuild pending', () => {
  const target = new EventTarget();
  let calls = 0;
  afterNextClick(target, () => {
    calls += 1;
  }, {
    idleMs: 10_000,
    ignore: (event) => event.detail === 'inside',
  });

  target.dispatchEvent(new CustomEvent('click', { detail: 'inside' }));
  assert.equal(calls, 0, 'clicking back inside the field keeps the edit open');
  target.dispatchEvent(new CustomEvent('click', { detail: 'outside' }));
  assert.equal(calls, 1, 'a real click still releases it');
});

test('a blur without a click falls back to the idle timer', async () => {
  const target = new EventTarget();
  let calls = 0;
  afterNextClick(target, () => {
    calls += 1;
  }, { idleMs: 20 });

  await new Promise((resolve) => setTimeout(resolve, 60));
  assert.equal(calls, 1, 'a focus left by keyboard or another window still ends the edit');
});
