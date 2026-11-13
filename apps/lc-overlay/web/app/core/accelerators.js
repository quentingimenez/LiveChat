// Keyboard accelerator capture and display (pure helpers, no DOM state).

// Converts a keydown event to a Tauri accelerator. Returns null while only
// modifiers are held, undefined for a bare key (it would capture typing in
// every application).
export function acceleratorFromEvent(event) {
  const key = event.key;
  if (['Control', 'Shift', 'Alt', 'Meta'].includes(key)) {
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
      if (/^num[0-9]$/.test(part)) {
        return `Num ${part.slice(3)}`;
      }
      return part;
    })
    .join(' + ');
}
