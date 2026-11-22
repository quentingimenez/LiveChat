// Keyboard accelerator capture and display (pure helpers, no DOM state).
//
// The capture follows the operating-system layout. On a French AZERTY keyboard
// the key labelled M is reported by `event.code` as "Semicolon", so deriving
// names from `code` rejects perfectly valid keys. Chromium sets `event.keyCode`
// to the Windows virtual-key code, which the OS layout already maps to the
// key's label: letters stay letters wherever they sit on the keyboard, and the
// punctuation keys keep the name Tauri's global-shortcut parser understands.
// `event.key` is only used to remember the character a punctuation key types
// (see shortcut-labels.js).

export const BARE_KEY_MESSAGE =
  'Utilise Ctrl, Alt ou Maj avec une lettre, un chiffre, une touche F, une flèche ou une touche spéciale.';

// global-hotkey has no name for a few Windows virtual keys (the French `!`/`§`
// key, dead keys, IME keys). The capture cannot produce an accelerator for
// them; the views show this message instead of pretending the modifier is
// missing.
export const UNSUPPORTED_KEY_MESSAGE =
  'Cette touche ne peut pas être utilisée comme raccourci (touche morte ou symbole non pris en charge par Windows).';

// Returned by acceleratorFromEvent for a key global-hotkey cannot register.
// A dedicated sentinel lets the views tell it apart from a bare key.
export const UNSUPPORTED_KEY = Symbol('unsupported-key');

// Windows only exposes "Ctrl+Alt" for AltGr in a global shortcut; right Alt is
// reported as the AltGraph modifier. Shared by the board and the settings page
// so both explain the same limitation.
export const ALTGR_HINT =
  'Alt droite (AltGr) est enregistrée comme Ctrl+Alt : Windows ne distingue pas Alt gauche et droite.';

// Chromium reports the right Alt of AltGr layouts as "AltGraph" (the UI Events
// modifier value); other platforms may expose it as "AltRight".
const MODIFIER_KEYS = ['Control', 'Shift', 'Alt', 'Meta', 'AltGraph', 'AltRight'];

// Virtual-key names accepted by global-hotkey (the parser normalises case).
const VK_NAMES = {
  8: 'Backspace',
  9: 'Tab',
  13: 'Enter',
  19: 'Pause',
  20: 'CapsLock',
  27: 'Escape',
  32: 'Space',
  33: 'PageUp',
  34: 'PageDown',
  35: 'End',
  36: 'Home',
  37: 'ArrowLeft',
  38: 'ArrowUp',
  39: 'ArrowRight',
  40: 'ArrowDown',
  44: 'PrintScreen',
  45: 'Insert',
  46: 'Delete',
  106: 'NumpadMultiply',
  107: 'NumpadAdd',
  109: 'NumpadSubtract',
  110: 'NumpadDecimal',
  111: 'NumpadDivide',
  144: 'NumLock',
  145: 'ScrollLock',
  173: 'AudioVolumeMute',
  174: 'AudioVolumeDown',
  175: 'AudioVolumeUp',
  176: 'MediaTrackNext',
  177: 'MediaTrackPrevious',
  178: 'MediaStop',
  179: 'MediaPlayPause',
  186: 'Semicolon',
  187: 'Equal',
  188: 'Comma',
  189: 'Minus',
  190: 'Period',
  191: 'Slash',
  192: 'Backquote',
  219: 'BracketLeft',
  220: 'Backslash',
  221: 'BracketRight',
  222: 'Quote',
};

const MODIFIER_LABELS = {
  CommandOrControl: 'Ctrl',
  Control: 'Ctrl',
  Ctrl: 'Ctrl',
  Shift: 'Maj',
  Alt: 'Alt',
  Option: 'Alt',
  Meta: 'Win',
  Command: 'Win',
  Cmd: 'Win',
  Super: 'Win',
};

const KEY_LABELS = {
  Escape: 'Échap',
  Space: 'Espace',
  Enter: 'Entrée',
  Tab: 'Tabulation',
  Backspace: 'Retour arrière',
  Delete: 'Suppr',
  Insert: 'Inser',
  Home: 'Début',
  End: 'Fin',
  PageUp: 'Page préc.',
  PageDown: 'Page suiv.',
  ArrowUp: 'Flèche haut',
  ArrowDown: 'Flèche bas',
  ArrowLeft: 'Flèche gauche',
  ArrowRight: 'Flèche droite',
  PrintScreen: 'Impr. écran',
  Pause: 'Pause',
  CapsLock: 'Verr. maj.',
  NumLock: 'Verr. num.',
  ScrollLock: 'Arr. défilé',
  NumpadMultiply: 'Num fois',
  NumpadAdd: 'Num plus',
  NumpadSubtract: 'Num moins',
  NumpadDecimal: 'Num point',
  NumpadDivide: 'Num divisé',
  AudioVolumeMute: 'Muet',
  AudioVolumeDown: 'Vol. -',
  AudioVolumeUp: 'Vol. +',
  MediaTrackNext: 'Piste suiv.',
  MediaTrackPrevious: 'Piste préc.',
  MediaStop: 'Stop',
  MediaPlayPause: 'Lecture/Pause',
  Semicolon: ';',
  Equal: '=',
  Comma: ',',
  Minus: '-',
  Period: '.',
  Slash: '/',
  Backquote: '`',
  BracketLeft: '[',
  Backslash: '\\',
  BracketRight: ']',
  Quote: "'",
};

// Escape pressed alone cancels a capture; with a modifier it is recorded.
export function cancelsCapture(event) {
  return event.key === 'Escape' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey;
}

// Windows virtual key → accelerator key name. Letters and digits are derived
// from their ranges; the rest comes from the table above.
function keyNameFromVirtualKey(keyCode) {
  if (keyCode >= 65 && keyCode <= 90) {
    return String.fromCharCode(keyCode);
  }
  if (keyCode >= 48 && keyCode <= 57) {
    return String.fromCharCode(keyCode);
  }
  if (keyCode >= 96 && keyCode <= 105) {
    return `num${keyCode - 96}`;
  }
  if (keyCode >= 112 && keyCode <= 135) {
    return `F${keyCode - 111}`;
  }
  return VK_NAMES[keyCode] || null;
}

// Converts a keydown event to a Tauri accelerator. Returns null while only
// modifiers are held, undefined for a bare key (it would capture typing in
// every application), UNSUPPORTED_KEY when a modifier is held but the key has
// no name global-hotkey can register.
export function acceleratorFromEvent(event) {
  const key = event.key;
  if (MODIFIER_KEYS.includes(key)) {
    return null;
  }
  const parts = [];
  if (event.ctrlKey || event.metaKey) {
    parts.push('CommandOrControl');
  }
  if (event.altKey) {
    parts.push('Alt');
  }
  if (event.shiftKey) {
    parts.push('Shift');
  }
  if (parts.length === 0) {
    return undefined;
  }
  if (key === 'Dead') {
    return UNSUPPORTED_KEY;
  }
  const name = keyNameFromVirtualKey(Number(event.keyCode));
  return name ? [...parts, name].join('+') : UNSUPPORTED_KEY;
}

// Human-readable accelerator: "CommandOrControl+Shift+S" → "Ctrl + Maj + S".
// `keyLabel` overrides the last part only: the views pass the character the
// user actually typed so an AZERTY punctuation key is not displayed with its
// US name ("Period" for `:`).
export function acceleratorLabel(accelerator, keyLabel = null) {
  if (!accelerator) {
    return null;
  }
  const parts = String(accelerator).split('+');
  return parts
    .map((part, index) => {
      if (index === parts.length - 1 && keyLabel) {
        return keyLabel;
      }
      const modifier = MODIFIER_LABELS[part];
      if (modifier) {
        return modifier;
      }
      const numpad = /^(?:num|Numpad)([0-9])$/.exec(part);
      if (numpad) {
        return `Num ${numpad[1]}`;
      }
      return KEY_LABELS[part] || part;
    })
    .join(' + ');
}
