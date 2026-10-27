interface KeyboardEventLike {
  ctrlKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  metaKey?: boolean;
  key?: string;
  code?: string;
}

export interface BoardLegacyShortcutUtils {
  acceleratorTokenToLabel(value: unknown): string;
  formatAcceleratorDisplay(accelerator: unknown): string;
  normalizeKeyFromCode(eventCode: unknown): string | null;
  normalizeKey(eventKey: unknown, eventCode: unknown): string | null;
  keyEventToDisplay(event: KeyboardEventLike): string;
  keyEventToAccelerator(event: KeyboardEventLike): string | null;
}

const CODE_TO_KEY_MAP: Record<string, string> = {
  Space: 'Space',
  Enter: 'Enter',
  Tab: 'Tab',
  Escape: 'Escape',
  Backspace: 'Backspace',
  Delete: 'Delete',
  Insert: 'Insert',
  Home: 'Home',
  End: 'End',
  PageUp: 'PageUp',
  PageDown: 'PageDown',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  Minus: '-',
  Equal: '=',
  Comma: ',',
  Period: '.',
  Slash: '/',
  Semicolon: ';',
  Quote: "'",
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Backquote: '`'
};

const KEY_TO_NORMALIZED_MAP: Record<string, string> = {
  ' ': 'Space',
  spacebar: 'Space',
  enter: 'Enter',
  return: 'Enter',
  tab: 'Tab',
  escape: 'Escape',
  esc: 'Escape',
  backspace: 'Backspace',
  delete: 'Delete',
  insert: 'Insert',
  home: 'Home',
  end: 'End',
  pageup: 'PageUp',
  pagedown: 'PageDown',
  arrowup: 'Up',
  arrowdown: 'Down',
  arrowleft: 'Left',
  arrowright: 'Right',
  plus: '+',
  minus: '-'
};

export function createBoardLegacyShortcutUtils(): BoardLegacyShortcutUtils {
  function acceleratorTokenToLabel(value: unknown): string {
    const token = `${value || ''}`.trim();
    const lowered = token.toLowerCase();

    if (lowered === 'commandorcontrol') {
      return 'Ctrl/Cmd';
    }

    if (lowered === 'super') {
      return 'Super';
    }

    if (lowered === 'alt') {
      return 'Alt';
    }

    if (lowered === 'shift') {
      return 'Shift';
    }

    if (token.startsWith('num')) {
      return `Num${token.slice(3)}`;
    }

    return token;
  }

  function formatAcceleratorDisplay(accelerator: unknown): string {
    const raw = `${accelerator || ''}`.trim();
    if (!raw) {
      return '';
    }

    return raw
      .split('+')
      .map((token) => acceleratorTokenToLabel(token))
      .filter(Boolean)
      .join(' + ');
  }

  function normalizeKeyFromCode(eventCode: unknown): string | null {
    const code = `${eventCode || ''}`;
    if (!code) {
      return null;
    }

    if (/^Key[A-Z]$/.test(code)) {
      return code.slice(3);
    }

    if (/^Digit[0-9]$/.test(code)) {
      return code.slice(5);
    }

    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) {
      return code;
    }

    if (/^Numpad[0-9]$/.test(code)) {
      return `num${code.slice(6)}`;
    }

    return CODE_TO_KEY_MAP[code] || null;
  }

  function normalizeKey(eventKey: unknown, eventCode: unknown): string | null {
    const codeCandidate = normalizeKeyFromCode(eventCode);
    if (codeCandidate) {
      return codeCandidate;
    }

    const key = `${eventKey || ''}`;
    if (!key) {
      return null;
    }

    const lower = key.toLowerCase();
    if (lower === 'control' || lower === 'shift' || lower === 'alt' || lower === 'meta') {
      return null;
    }

    if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(key)) {
      return key.toUpperCase();
    }

    if (/^[a-z]$/i.test(key)) {
      return key.toUpperCase();
    }

    if (/^[0-9]$/.test(key)) {
      return key;
    }

    return KEY_TO_NORMALIZED_MAP[lower] || null;
  }

  function keyEventToDisplay(event: KeyboardEventLike): string {
    const parts: string[] = [];

    if (event.ctrlKey) {
      parts.push('Ctrl/Cmd');
    }

    if (event.altKey) {
      parts.push('Alt');
    }

    if (event.shiftKey) {
      parts.push('Shift');
    }

    if (event.metaKey) {
      parts.push('Super');
    }

    const key = normalizeKey(event.key, event.code);
    if (key) {
      parts.push(acceleratorTokenToLabel(key));
    }

    return parts.join(' + ');
  }

  function keyEventToAccelerator(event: KeyboardEventLike): string | null {
    const key = normalizeKey(event.key, event.code);
    if (!key) {
      return null;
    }

    const modifiers: string[] = [];

    if (event.ctrlKey) {
      modifiers.push('CommandOrControl');
    }

    if (event.altKey) {
      modifiers.push('Alt');
    }

    if (event.shiftKey) {
      modifiers.push('Shift');
    }

    if (event.metaKey) {
      modifiers.push('Super');
    }

    if (modifiers.length === 0) {
      return null;
    }

    return [...modifiers, key].join('+');
  }

  return {
    acceleratorTokenToLabel,
    formatAcceleratorDisplay,
    normalizeKeyFromCode,
    normalizeKey,
    keyEventToDisplay,
    keyEventToAccelerator
  };
}
