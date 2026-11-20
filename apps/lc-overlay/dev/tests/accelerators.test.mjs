// Unit tests of the keyboard accelerator helpers (Node's built-in runner).

import assert from 'node:assert/strict';
import test from 'node:test';

import { acceleratorFromEvent, acceleratorLabel, cancelsCapture } from '../../web/app/core/accelerators.js';

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
  assert.equal(acceleratorFromEvent(keyEvent({ shiftKey: true, key: 'Escape', code: 'Escape' })), 'Shift+Escape');
  assert.equal(acceleratorFromEvent(keyEvent({ ctrlKey: true, altKey: true, key: 'Escape', code: 'Escape' })), 'CommandOrControl+Alt+Escape');
});

test('acceleratorFromEvent treats right Alt / AltGr as a modifier', () => {
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'AltGraph', code: 'AltRight', ctrlKey: true, altKey: true })),
    null,
  );
  assert.equal(acceleratorFromEvent(keyEvent({ key: 'AltRight', code: 'AltRight', altKey: true })), null);
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'æ', code: 'KeyA', ctrlKey: true, altKey: true })),
    'CommandOrControl+Alt+A',
  );
});

test('cancelsCapture only accepts Escape pressed alone', () => {
  assert.equal(cancelsCapture(keyEvent({ key: 'Escape', code: 'Escape' })), true);
  assert.equal(cancelsCapture(keyEvent({ shiftKey: true, key: 'Escape', code: 'Escape' })), false);
  assert.equal(cancelsCapture(keyEvent({ ctrlKey: true, key: 'Escape', code: 'Escape' })), false);
  assert.equal(cancelsCapture(keyEvent({ altKey: true, key: 'Escape', code: 'Escape' })), false);
  assert.equal(cancelsCapture(keyEvent({})), false);
});

test('acceleratorFromEvent waits on modifiers and refuses bare keys', () => {
  assert.equal(acceleratorFromEvent(keyEvent({ key: 'Control', code: 'ControlLeft', ctrlKey: true })), null);
  assert.equal(acceleratorFromEvent(keyEvent({})), undefined);
  assert.equal(acceleratorFromEvent(keyEvent({ key: 'Escape', code: 'Escape' })), undefined);
});

test('acceleratorLabel renders a readable accelerator', () => {
  assert.equal(acceleratorLabel('CommandOrControl+Shift+S'), 'Ctrl + Maj + S');
  assert.equal(acceleratorLabel('Alt+F5'), 'Alt + F5');
  assert.equal(acceleratorLabel('CommandOrControl+num2'), 'Ctrl + Num 2');
  assert.equal(acceleratorLabel('Shift+Escape'), 'Maj + Échap');
  assert.equal(acceleratorLabel(null), null);
});
