// Layout-dependent labels for keyboard shortcuts.
//
// An accelerator stores a global-hotkey key name, which is derived from the
// Windows virtual-key code. That name is stable but does not always match the
// character printed on the key: on French AZERTY, `:` is stored as
// "CommandOrControl+Period" (VK_OEM_PERIOD) while the user reads "Ctrl + :".
// The capture remembers the character the user typed for this machine and the
// views show it, falling back to the canonical accelerator label when there is
// no remembered character.
//
// The cache lives in localStorage because labels are device and layout
// specific, not part of the synced overlay configuration. Storage is injected
// (localStorage in the app, a fake Map in the tests); a missing or broken
// storage simply disables the cache.

import { acceleratorLabel } from './accelerators.js';

export const SHORTCUT_LABEL_STORAGE_KEY = 'livechat.shortcut-labels.v1';
export const SHORTCUT_LABEL_VERSION = 1;
// Accelerators are a small, bounded set; this only guards against a corrupted
// or hand-edited storage growing without limit.
export const MAX_SHORTCUT_LABELS = 200;
const MAX_ACCELERATOR_LENGTH = 80;
const MAX_LABEL_LENGTH = 8;

function defaultStorage() {
  try {
    return globalThis.localStorage || null;
  } catch {
    return null;
  }
}

function emptyStore() {
  return { version: SHORTCUT_LABEL_VERSION, labels: {} };
}

function validAccelerator(accelerator) {
  return (
    typeof accelerator === 'string' &&
    accelerator.length > 0 &&
    accelerator.length <= MAX_ACCELERATOR_LENGTH
  );
}

// Labels are single characters in practice (the captured `event.key`). Keeping
// them short and free of `+` preserves the readability of "Ctrl + <label>".
function validLabel(label) {
  return (
    typeof label === 'string' &&
    label.length > 0 &&
    label.length <= MAX_LABEL_LENGTH &&
    label === label.trim() &&
    !label.includes('+') &&
    !/[\u0000-\u001f\u007f]/.test(label)
  );
}

function trimLabels(labels) {
  const keys = Object.keys(labels);
  if (keys.length > MAX_SHORTCUT_LABELS) {
    for (const key of keys.slice(0, keys.length - MAX_SHORTCUT_LABELS)) {
      delete labels[key];
    }
  }
  return labels;
}

function normalizeStore(parsed) {
  if (
    !parsed ||
    parsed.version !== SHORTCUT_LABEL_VERSION ||
    !parsed.labels ||
    typeof parsed.labels !== 'object' ||
    Array.isArray(parsed.labels)
  ) {
    return emptyStore();
  }
  const labels = {};
  for (const [accelerator, label] of Object.entries(parsed.labels)) {
    if (validAccelerator(accelerator) && validLabel(label)) {
      labels[accelerator] = label;
    }
  }
  return { version: SHORTCUT_LABEL_VERSION, labels: trimLabels(labels) };
}

function readStore(storage) {
  if (!storage) {
    return emptyStore();
  }
  try {
    const raw = storage.getItem(SHORTCUT_LABEL_STORAGE_KEY);
    return raw ? normalizeStore(JSON.parse(raw)) : emptyStore();
  } catch {
    return emptyStore();
  }
}

function writeStore(storage, store) {
  if (!storage) {
    return false;
  }
  try {
    storage.setItem(SHORTCUT_LABEL_STORAGE_KEY, JSON.stringify(store));
    return true;
  } catch {
    // Quota exceeded, private mode or a disabled storage: the canonical
    // accelerator label is still correct enough, never break the UI.
    return false;
  }
}

// Label previously captured for this accelerator, or null.
export function readShortcutLabel(accelerator, storage = defaultStorage()) {
  if (!validAccelerator(accelerator)) {
    return null;
  }
  const label = readStore(storage).labels[accelerator];
  return validLabel(label) ? label : null;
}

// Remembers the character typed for `accelerator`; refreshes its recency.
export function writeShortcutLabel(accelerator, label, storage = defaultStorage()) {
  if (!validAccelerator(accelerator) || !validLabel(label)) {
    return false;
  }
  const store = readStore(storage);
  delete store.labels[accelerator];
  store.labels[accelerator] = label;
  trimLabels(store.labels);
  return writeStore(storage, store);
}

// Removes a stored label. Used when a new capture of the same accelerator typed
// a plain letter or digit: the old punctuation character would otherwise stay
// displayed for a binding that is no longer that punctuation key.
export function forgetShortcutLabel(accelerator, storage = defaultStorage()) {
  if (!validAccelerator(accelerator)) {
    return false;
  }
  const store = readStore(storage);
  if (!(accelerator in store.labels)) {
    return false;
  }
  delete store.labels[accelerator];
  return writeStore(storage, store);
}

// Display label of a shortcut, using the captured character when there is one.
export function shortcutLabel(accelerator, storage = defaultStorage()) {
  if (!accelerator) {
    return null;
  }
  return acceleratorLabel(accelerator, readShortcutLabel(accelerator, storage));
}

// Character to remember for a captured keydown, or null when the canonical
// accelerator label is already the right one. ASCII letters and digits render
// as themselves; AltGr on a letter key is recorded as Ctrl+Alt+<letter>, so the
// letter stays the readable part.
export function capturedKeyLabel(event) {
  const key = event.key;
  if (!key || key === 'Dead' || key === 'Unidentified' || key === 'Process') {
    return null;
  }
  if ([...key].length !== 1 || /\s/.test(key)) {
    return null;
  }
  if (/^[A-Za-z0-9]$/.test(key)) {
    return null;
  }
  if (event.ctrlKey && event.altKey && /\p{L}/u.test(key)) {
    return null;
  }
  return key;
}
