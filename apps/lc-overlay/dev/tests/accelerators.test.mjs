// Unit tests of the keyboard accelerator helpers (Node's built-in runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import { acceleratorFromEvent, acceleratorLabel } from '../../web/app/core/accelerators.js';

function keyEvent(overrides) {
  return {
    key: 's',
    code: 'KeyS',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...overrides,
  };
}

test('acceleratorFromEvent builds a Tauri accelerator from a keydown event', () => {
  assert.equal(acceleratorFromEvent(keyEvent({ ctrlKey: true, shiftKey: true })), 'CommandOrControl+Shift+S');
  assert.equal(acceleratorFromEvent(keyEvent({ metaKey: true, key: '1', code: 'Digit1' })), 'CommandOrControl+1');
  assert.equal(acceleratorFromEvent(keyEvent({ altKey: true, key: 'F5', code: 'F5' })), 'Alt+F5');
  assert.equal(acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '2', code: 'Numpad2' })), 'CommandOrControl+num2');
});

test('acceleratorFromEvent waits on modifiers and refuses bare keys', () => {
  assert.equal(acceleratorFromEvent(keyEvent({ key: 'Control', code: 'ControlLeft', ctrlKey: true })), null);
  assert.equal(acceleratorFromEvent(keyEvent({})), undefined);
});

test('acceleratorLabel renders a readable accelerator', () => {
  assert.equal(acceleratorLabel('CommandOrControl+Shift+S'), 'Ctrl + Maj + S');
  assert.equal(acceleratorLabel('Alt+F5'), 'Alt + F5');
  assert.equal(acceleratorLabel('CommandOrControl+num2'), 'Ctrl + Num 2');
  assert.equal(acceleratorLabel(null), null);
});
