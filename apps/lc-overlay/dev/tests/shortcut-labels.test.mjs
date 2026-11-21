// Unit tests of the layout-dependent shortcut labels (Node's built-in runner).
//
// The cache only exists to display the character a punctuation key actually
// types on this keyboard; the accelerator itself stays the device-independent
// global-hotkey name. The tests inject a fake localStorage so they never touch
// the environment.

import assert from 'node:assert/strict';
import test from 'node:test';

import { acceleratorFromEvent } from '../../web/app/core/accelerators.js';
import {
  capturedKeyLabel,
  forgetShortcutLabel,
  MAX_SHORTCUT_LABELS,
  readShortcutLabel,
  SHORTCUT_LABEL_STORAGE_KEY,
  SHORTCUT_LABEL_VERSION,
  shortcutLabel,
  writeShortcutLabel,
} from '../../web/app/core/shortcut-labels.js';

function fakeStorage(entries = {}) {
  const data = new Map(Object.entries(entries));
  return {
    getItem: (key) => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    removeItem: (key) => data.delete(key),
  };
}

function storedLabels(labels) {
  return JSON.stringify({ version: SHORTCUT_LABEL_VERSION, labels });
}

test('writeShortcutLabel remembers the typed character and shortcutLabel uses it', () => {
  const storage = fakeStorage();
  assert.equal(writeShortcutLabel('CommandOrControl+Shift+Slash', ':', storage), true);
  assert.equal(readShortcutLabel('CommandOrControl+Shift+Slash', storage), ':');
  assert.equal(shortcutLabel('CommandOrControl+Shift+Slash', storage), 'Ctrl + Maj + :');
  // Without a stored label the canonical accelerator name is displayed.
  assert.equal(shortcutLabel('CommandOrControl+Shift+Slash', fakeStorage()), 'Ctrl + Maj + /');
  assert.equal(shortcutLabel(null, storage), null);
});

test('forgetShortcutLabel drops a stale captured character', () => {
  const storage = fakeStorage();
  assert.equal(writeShortcutLabel('CommandOrControl+2', 'é', storage), true);
  assert.equal(shortcutLabel('CommandOrControl+2', storage), 'Ctrl + é');
  assert.equal(forgetShortcutLabel('CommandOrControl+2', storage), true);
  assert.equal(readShortcutLabel('CommandOrControl+2', storage), null);
  assert.equal(shortcutLabel('CommandOrControl+2', storage), 'Ctrl + 2');
  assert.equal(forgetShortcutLabel('CommandOrControl+2', storage), false);
  assert.equal(forgetShortcutLabel(null, storage), false);
});

test('invalid storage is ignored and rewritten', () => {
  const broken = fakeStorage({ [SHORTCUT_LABEL_STORAGE_KEY]: '{' });
  assert.equal(readShortcutLabel('CommandOrControl+Period', broken), null);
  assert.equal(writeShortcutLabel('CommandOrControl+Period', ':', broken), true);
  assert.equal(readShortcutLabel('CommandOrControl+Period', broken), ':');

  const wrongVersion = fakeStorage({
    [SHORTCUT_LABEL_STORAGE_KEY]: JSON.stringify({
      version: SHORTCUT_LABEL_VERSION + 1,
      labels: { 'CommandOrControl+Period': ':' },
    }),
  });
  assert.equal(readShortcutLabel('CommandOrControl+Period', wrongVersion), null);

  const notAnObject = fakeStorage({
    [SHORTCUT_LABEL_STORAGE_KEY]: storedLabels([]),
  });
  assert.equal(readShortcutLabel('CommandOrControl+Period', notAnObject), null);
});

test('invalid stored entries are dropped and pruned on the next write', () => {
  const storage = fakeStorage({
    [SHORTCUT_LABEL_STORAGE_KEY]: storedLabels({
      'CommandOrControl+Period': ':',
      'CommandOrControl+Comma': 42,
      'CommandOrControl+Semicolon': '',
      'CommandOrControl+Slash': 'a+b',
      'CommandOrControl+Backslash': 'toolonglabel',
      'CommandOrControl+Quote': ' x',
    }),
  });
  assert.equal(readShortcutLabel('CommandOrControl+Period', storage), ':');
  for (const accelerator of [
    'CommandOrControl+Comma',
    'CommandOrControl+Semicolon',
    'CommandOrControl+Slash',
    'CommandOrControl+Backslash',
    'CommandOrControl+Quote',
  ]) {
    assert.equal(readShortcutLabel(accelerator, storage), null, accelerator);
  }
  assert.equal(writeShortcutLabel('Alt+F5', 'é', storage), true);
  const persisted = JSON.parse(storage.getItem(SHORTCUT_LABEL_STORAGE_KEY));
  assert.deepEqual(Object.keys(persisted.labels).sort(), [
    'Alt+F5',
    'CommandOrControl+Period',
  ]);
});

test('writeShortcutLabel rejects malformed keys and labels', () => {
  const storage = fakeStorage();
  for (const label of ['', 'a+b', ' x', 'toolonglabel', 42, null]) {
    assert.equal(writeShortcutLabel('CommandOrControl+Period', label, storage), false);
  }
  assert.equal(writeShortcutLabel('', ':', storage), false);
  assert.equal(writeShortcutLabel(null, ':', storage), false);
  assert.equal(storage.getItem(SHORTCUT_LABEL_STORAGE_KEY), null);
});

test('the label cache keeps only the most recent entries', () => {
  const storage = fakeStorage();
  const accelerator = (index) => `Test+${index}`;
  for (let index = 0; index < MAX_SHORTCUT_LABELS; index += 1) {
    assert.equal(writeShortcutLabel(accelerator(index), 'a', storage), true);
  }
  // Refresh the oldest entry, then add five more: it must survive while the
  // next oldest ones are evicted.
  assert.equal(writeShortcutLabel(accelerator(0), 'b', storage), true);
  for (let index = MAX_SHORTCUT_LABELS; index < MAX_SHORTCUT_LABELS + 5; index += 1) {
    writeShortcutLabel(accelerator(index), 'a', storage);
  }
  const persisted = JSON.parse(storage.getItem(SHORTCUT_LABEL_STORAGE_KEY));
  assert.equal(Object.keys(persisted.labels).length, MAX_SHORTCUT_LABELS);
  assert.equal(persisted.labels[accelerator(0)], 'b');
  assert.equal(persisted.labels[accelerator(1)], undefined);
  assert.equal(persisted.labels[accelerator(MAX_SHORTCUT_LABELS + 4)], 'a');
});

test('capturedKeyLabel keeps layout punctuation but not letters or dead keys', () => {
  assert.equal(capturedKeyLabel({ key: '&', keyCode: 49 }), '&');
  assert.equal(capturedKeyLabel({ key: 'é', keyCode: 50 }), 'é');
  assert.equal(capturedKeyLabel({ key: 'Ф', keyCode: 65 }), 'Ф');
  // AltGr punctuation keeps its typed character, AltGr letters keep the key.
  assert.equal(capturedKeyLabel({ key: '£', keyCode: 219, ctrlKey: true, altKey: true }), '£');
  assert.equal(capturedKeyLabel({ key: 'æ', keyCode: 65, ctrlKey: true, altKey: true }), null);
  assert.equal(capturedKeyLabel({ key: 'm', keyCode: 77 }), null);
  assert.equal(capturedKeyLabel({ key: 'M', keyCode: 77, shiftKey: true }), null);
  assert.equal(capturedKeyLabel({ key: '1', keyCode: 49, shiftKey: true }), null);
  assert.equal(capturedKeyLabel({ key: 'Dead', keyCode: 221 }), null);
  assert.equal(capturedKeyLabel({ key: ' ', keyCode: 32 }), null);
  assert.equal(capturedKeyLabel({ key: 'Escape', keyCode: 27 }), null);
  assert.equal(capturedKeyLabel({ key: 'Unidentified', keyCode: 0 }), null);
  assert.equal(capturedKeyLabel({ key: '' }), null);
});

test('a captured AZERTY punctuation key displays its character end to end', () => {
  const event = {
    key: ':',
    code: 'Period',
    keyCode: 190,
    ctrlKey: true,
    shiftKey: false,
    altKey: false,
    metaKey: false,
  };
  const accelerator = acceleratorFromEvent(event);
  // The accelerator is the US name of VK_OEM_PERIOD, whose default label is
  // "."; the cache keeps the character the French key actually types.
  assert.equal(accelerator, 'CommandOrControl+Period');
  const storage = fakeStorage();
  assert.equal(writeShortcutLabel(accelerator, capturedKeyLabel(event), storage), true);
  assert.equal(shortcutLabel(accelerator, storage), 'Ctrl + :');
});
