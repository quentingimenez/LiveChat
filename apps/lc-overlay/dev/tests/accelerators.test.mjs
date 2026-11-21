// Unit tests of the keyboard accelerator helpers (Node's built-in runner).
//
// The fixtures describe a French AZERTY keyboard: the key labels come from the
// Windows virtual-key codes while `code` keeps the US physical position, which
// is exactly what WebView2 reports.

import assert from 'node:assert/strict';
import test from 'node:test';

import {
  acceleratorFromEvent,
  acceleratorLabel,
  cancelsCapture,
  UNSUPPORTED_KEY,
} from '../../web/app/core/accelerators.js';

function keyEvent(overrides) {
  return {
    key: 's',
    code: 'KeyS',
    keyCode: 83,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...overrides,
  };
}

test('acceleratorFromEvent follows the OS layout for letters', () => {
  // The key labelled M sits on the US ";" position: `code` says Semicolon but
  // the Windows virtual key is VK_M. This is the historical bug: Ctrl+M was
  // rejected as "Semicolon".
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'm', code: 'Semicolon', keyCode: 77 })),
    'CommandOrControl+M',
  );
  // AZERTY A and Z sit on the US Q and W positions.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'a', code: 'KeyQ', keyCode: 65 })),
    'CommandOrControl+A',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'z', code: 'KeyW', keyCode: 90 })),
    'CommandOrControl+Z',
  );
  // A QWERTY key keeps its name.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 's', code: 'KeyS', keyCode: 83 })),
    'CommandOrControl+S',
  );
});

test('acceleratorFromEvent keeps the virtual key of AZERTY digits and punctuation', () => {
  // "&" is the unshifted character of the 1 key: the accelerator is Ctrl+1.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '&', code: 'Digit1', keyCode: 49 })),
    'CommandOrControl+1',
  );
  // Typing the digit 1 needs Shift, so the accelerator keeps Shift.
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ ctrlKey: true, shiftKey: true, key: '1', code: 'Digit1', keyCode: 49 }),
    ),
    'CommandOrControl+Shift+1',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'é', code: 'Digit2', keyCode: 50 })),
    'CommandOrControl+2',
  );
  // On AZERTY, "." is Shift+";" and ":" is the unshifted character of the
  // US "." key; "," sits on the US M position.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: ';', code: 'Comma', keyCode: 186 })),
    'CommandOrControl+Semicolon',
  );
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ ctrlKey: true, shiftKey: true, key: '.', code: 'Comma', keyCode: 186 }),
    ),
    'CommandOrControl+Shift+Semicolon',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: ',', code: 'KeyM', keyCode: 188 })),
    'CommandOrControl+Comma',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: ':', code: 'Period', keyCode: 190 })),
    'CommandOrControl+Period',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '²', code: 'Backquote', keyCode: 222 })),
    'CommandOrControl+Quote',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'ù', code: 'Quote', keyCode: 192 })),
    'CommandOrControl+Backquote',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '$', code: 'BracketLeft', keyCode: 219 })),
    'CommandOrControl+BracketLeft',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '*', code: 'Backslash', keyCode: 220 })),
    'CommandOrControl+Backslash',
  );
});

test('acceleratorFromEvent reports keys global-hotkey cannot register', () => {
  // The French "!"/"§" key is VK_OEM_8, which global-hotkey does not map.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '!', code: 'Slash', keyCode: 223 })),
    UNSUPPORTED_KEY,
  );
  // A dead key is not a stable character even though its virtual key has a
  // name (VK_OEM_6 here).
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ ctrlKey: true, key: 'Dead', code: 'BracketRight', keyCode: 221 }),
    ),
    UNSUPPORTED_KEY,
  );
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ ctrlKey: true, key: 'Unidentified', code: 'Unidentified', keyCode: 0 }),
    ),
    UNSUPPORTED_KEY,
  );
});

test('acceleratorFromEvent accepts named keys, arrows, function keys and the numpad', () => {
  const named = [
    [8, 'Backspace'],
    [9, 'Tab'],
    [13, 'Enter'],
    [19, 'Pause'],
    [20, 'CapsLock'],
    [27, 'Escape'],
    [32, 'Space'],
    [33, 'PageUp'],
    [34, 'PageDown'],
    [35, 'End'],
    [36, 'Home'],
    [37, 'ArrowLeft'],
    [38, 'ArrowUp'],
    [39, 'ArrowRight'],
    [40, 'ArrowDown'],
    [44, 'PrintScreen'],
    [45, 'Insert'],
    [46, 'Delete'],
    [106, 'NumpadMultiply'],
    [107, 'NumpadAdd'],
    [109, 'NumpadSubtract'],
    [110, 'NumpadDecimal'],
    [111, 'NumpadDivide'],
    [144, 'NumLock'],
    [145, 'ScrollLock'],
    [173, 'AudioVolumeMute'],
    [174, 'AudioVolumeDown'],
    [175, 'AudioVolumeUp'],
    [176, 'MediaTrackNext'],
    [177, 'MediaTrackPrevious'],
    [178, 'MediaStop'],
    [179, 'MediaPlayPause'],
  ];
  for (const [keyCode, name] of named) {
    assert.equal(
      acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'x', code: 'KeyX', keyCode })),
      `CommandOrControl+${name}`,
      `keyCode ${keyCode}`,
    );
  }
  for (const [keyCode, name] of [
    [96, 'num0'],
    [97, 'num1'],
    [105, 'num9'],
  ]) {
    assert.equal(
      acceleratorFromEvent(keyEvent({ ctrlKey: true, key: '1', code: 'Numpad1', keyCode })),
      `CommandOrControl+${name}`,
    );
  }
  for (const [keyCode, name] of [
    [112, 'F1'],
    [123, 'F12'],
    [124, 'F13'],
    [135, 'F24'],
  ]) {
    assert.equal(
      acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'F1', code: 'F1', keyCode })),
      `CommandOrControl+${name}`,
    );
  }
  // Numpad Enter shares VK_RETURN with the main Enter key.
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, key: 'Enter', code: 'NumpadEnter', keyCode: 13 })),
    'CommandOrControl+Enter',
  );
});

test('acceleratorFromEvent builds the modifier prefixes', () => {
  assert.equal(
    acceleratorFromEvent(keyEvent({ ctrlKey: true, shiftKey: true })),
    'CommandOrControl+Shift+S',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ metaKey: true, key: '1', code: 'Digit1', keyCode: 49 })),
    'CommandOrControl+1',
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ altKey: true, key: 'F5', code: 'F5', keyCode: 116 })),
    'Alt+F5',
  );
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ ctrlKey: true, altKey: true, shiftKey: true, key: '2', code: 'Numpad2', keyCode: 98 }),
    ),
    'CommandOrControl+Alt+Shift+num2',
  );
});

test('acceleratorFromEvent treats right Alt / AltGr as a modifier', () => {
  assert.equal(
    acceleratorFromEvent(
      keyEvent({ key: 'AltGraph', code: 'AltRight', ctrlKey: true, altKey: true }),
    ),
    null,
  );
  assert.equal(acceleratorFromEvent(keyEvent({ key: 'AltRight', code: 'AltRight', altKey: true })), null);
  // French AltGr+A types "æ" on the key labelled A (US Q position); the virtual
  // key is still A.
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'æ', code: 'KeyQ', keyCode: 65, ctrlKey: true, altKey: true })),
    'CommandOrControl+Alt+A',
  );
  // AltGr+E (€) keeps the E virtual key.
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: '€', code: 'KeyE', keyCode: 69, ctrlKey: true, altKey: true })),
    'CommandOrControl+Alt+E',
  );
});

test('cancelsCapture only accepts Escape pressed alone', () => {
  assert.equal(cancelsCapture(keyEvent({ key: 'Escape', code: 'Escape', keyCode: 27 })), true);
  assert.equal(
    cancelsCapture(keyEvent({ shiftKey: true, key: 'Escape', code: 'Escape', keyCode: 27 })),
    false,
  );
  assert.equal(
    cancelsCapture(keyEvent({ ctrlKey: true, key: 'Escape', code: 'Escape', keyCode: 27 })),
    false,
  );
  assert.equal(
    cancelsCapture(keyEvent({ altKey: true, key: 'Escape', code: 'Escape', keyCode: 27 })),
    false,
  );
  assert.equal(cancelsCapture(keyEvent({})), false);
});

test('acceleratorFromEvent waits on modifiers and refuses bare keys', () => {
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'Control', code: 'ControlLeft', keyCode: 17, ctrlKey: true })),
    null,
  );
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'Meta', code: 'MetaLeft', keyCode: 91, metaKey: true })),
    null,
  );
  assert.equal(acceleratorFromEvent(keyEvent({})), undefined);
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: 'Escape', code: 'Escape', keyCode: 27 })),
    undefined,
  );
  // Without a modifier, an unsupported key is a bare key, not a registration
  // error.
  assert.equal(
    acceleratorFromEvent(keyEvent({ key: '!', code: 'Slash', keyCode: 223 })),
    undefined,
  );
});

test('acceleratorLabel renders a readable accelerator', () => {
  assert.equal(acceleratorLabel('CommandOrControl+Shift+S'), 'Ctrl + Maj + S');
  assert.equal(acceleratorLabel('Alt+F5'), 'Alt + F5');
  assert.equal(acceleratorLabel('CommandOrControl+num2'), 'Ctrl + Num 2');
  assert.equal(acceleratorLabel('CommandOrControl+Numpad2'), 'Ctrl + Num 2');
  assert.equal(acceleratorLabel('Shift+Escape'), 'Maj + Échap');
  assert.equal(acceleratorLabel('CommandOrControl+Space'), 'Ctrl + Espace');
  assert.equal(acceleratorLabel('CommandOrControl+ArrowLeft'), 'Ctrl + Flèche gauche');
  assert.equal(acceleratorLabel('CommandOrControl+Semicolon'), 'Ctrl + ;');
  assert.equal(acceleratorLabel('CommandOrControl+NumpadAdd'), 'Ctrl + Num plus');
  assert.equal(acceleratorLabel('Control+Alt+Delete'), 'Ctrl + Alt + Suppr');
  assert.equal(acceleratorLabel('Super+F12'), 'Win + F12');
  assert.equal(acceleratorLabel(null), null);
});

test('acceleratorLabel lets the captured character replace the key label only', () => {
  assert.equal(acceleratorLabel('CommandOrControl+Shift+Slash', ':'), 'Ctrl + Maj + :');
  assert.equal(acceleratorLabel('Alt+F5', 'x'), 'Alt + x');
  assert.equal(acceleratorLabel('CommandOrControl+Shift+S', null), 'Ctrl + Maj + S');
});
