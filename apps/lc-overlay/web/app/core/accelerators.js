// Keyboard accelerator capture and display (pure helpers, no DOM state).

export const BARE_KEY_MESSAGE = 'Utilise Ctrl, Alt ou Maj avec une lettre, un chiffre, une touche F ou Échap.';

// Windows only exposes "Ctrl+Alt" for AltGr in a global shortcut; right Alt is
// reported as the AltGraph modifier. Shared by the board and the settings page
// so both explain the same limitation.
export const ALTGR_HINT =
  'Alt droite (AltGr) est enregistrée comme Ctrl+Alt : Windows ne distingue pas Alt gauche et droite.';

// Chromium reports the right Alt of AltGr layouts as "AltGraph" (the UI Events
// modifier value); other platforms may expose it as "AltRight".
const MODIFIER_KEYS = ['Control', 'Shift', 'Alt', 'Meta', 'AltGraph', 'AltRight'];

// Escape pressed alone cancels a capture; with a modifier it is recorded.
export function cancelsCapture(event) {
  return event.key === 'Escape' && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey;
}

// Converts a keydown event to a Tauri accelerator. Returns null while only
// modifiers are held, undefined for a bare key (it would capture typing in
// every application).
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
  let name = null;
  if (/^Key[A-Z]$/.test(event.code)) {
    name = event.code.slice(3);
  } else if (/^Digit[0-9]$/.test(event.code)) {
    name = event.code.slice(5);
  } else if (/^Numpad[0-9]$/.test(event.code)) {
    name = `num${event.code.slice(6)}`;
  } else if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) {
    name = key;
  } else if (key === 'Escape') {
    name = 'Escape';
  }
  return name ? [...parts, name].join('+') : undefined;
}

// Human-readable accelerator: "CommandOrControl+Shift+S" → "Ctrl + Maj + S".
export function acceleratorLabel(accelerator) {
  if (!accelerator) {
    return null;
  }
  return String(accelerator)
    .split('+')
    .map((part) => {
      if (part === 'CommandOrControl') {
        return 'Ctrl';
      }
      if (part === 'Shift') {
        return 'Maj';
      }
      if (part === 'Escape') {
        return 'Échap';
      }
      if (/^num[0-9]$/.test(part)) {
        return `Num ${part.slice(3)}`;
      }
      return part;
    })
    .join(' + ');
}
