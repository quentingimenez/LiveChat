import { describe, it, expect } from 'vitest';
import { toNonEmptyString, toBooleanFlag, formatBytesForMessage } from './stringUtils';

// ---------------------------------------------------------------------------
// toNonEmptyString
// ---------------------------------------------------------------------------
describe('toNonEmptyString', () => {
  it('returns trimmed string for normal input', () => {
    expect(toNonEmptyString('hello')).toBe('hello');
  });

  it('trims leading and trailing whitespace', () => {
    expect(toNonEmptyString('  hello  ')).toBe('hello');
  });

  it('returns null for empty string', () => {
    expect(toNonEmptyString('')).toBeNull();
  });

  it('returns null for whitespace-only string', () => {
    expect(toNonEmptyString('   ')).toBeNull();
    expect(toNonEmptyString('\t\n')).toBeNull();
  });

  it('returns null for non-string types', () => {
    expect(toNonEmptyString(null)).toBeNull();
    expect(toNonEmptyString(undefined)).toBeNull();
    expect(toNonEmptyString(0)).toBeNull();
    expect(toNonEmptyString(42)).toBeNull();
    expect(toNonEmptyString(false)).toBeNull();
    expect(toNonEmptyString(true)).toBeNull();
    expect(toNonEmptyString({})).toBeNull();
    expect(toNonEmptyString([])).toBeNull();
  });

  it('preserves internal whitespace', () => {
    expect(toNonEmptyString('hello world')).toBe('hello world');
    expect(toNonEmptyString('  a  b  ')).toBe('a  b');
  });
});

// ---------------------------------------------------------------------------
// toBooleanFlag
// ---------------------------------------------------------------------------
describe('toBooleanFlag', () => {
  describe('boolean input', () => {
    it('returns true for true', () => {
      expect(toBooleanFlag(true)).toBe(true);
    });

    it('returns false for false', () => {
      expect(toBooleanFlag(false)).toBe(false);
    });
  });

  describe('number input', () => {
    it('returns true for 1', () => {
      expect(toBooleanFlag(1)).toBe(true);
    });

    it('returns false for 0', () => {
      expect(toBooleanFlag(0)).toBe(false);
    });

    it('returns false for other numbers', () => {
      expect(toBooleanFlag(2)).toBe(false);
      expect(toBooleanFlag(-1)).toBe(false);
      expect(toBooleanFlag(0.5)).toBe(false);
    });
  });

  describe('string input', () => {
    it('returns true for truthy strings', () => {
      expect(toBooleanFlag('1')).toBe(true);
      expect(toBooleanFlag('true')).toBe(true);
      expect(toBooleanFlag('yes')).toBe(true);
      expect(toBooleanFlag('on')).toBe(true);
    });

    it('is case-insensitive', () => {
      expect(toBooleanFlag('TRUE')).toBe(true);
      expect(toBooleanFlag('True')).toBe(true);
      expect(toBooleanFlag('YES')).toBe(true);
      expect(toBooleanFlag('On')).toBe(true);
    });

    it('trims whitespace', () => {
      expect(toBooleanFlag('  true  ')).toBe(true);
      expect(toBooleanFlag(' 1 ')).toBe(true);
    });

    it('returns false for falsy strings', () => {
      expect(toBooleanFlag('0')).toBe(false);
      expect(toBooleanFlag('false')).toBe(false);
      expect(toBooleanFlag('no')).toBe(false);
      expect(toBooleanFlag('off')).toBe(false);
      expect(toBooleanFlag('')).toBe(false);
      expect(toBooleanFlag('random')).toBe(false);
    });
  });

  describe('other types', () => {
    it('returns false for null/undefined', () => {
      expect(toBooleanFlag(null)).toBe(false);
      expect(toBooleanFlag(undefined)).toBe(false);
    });

    it('returns false for objects and arrays', () => {
      expect(toBooleanFlag({})).toBe(false);
      expect(toBooleanFlag([])).toBe(false);
    });
  });
});

// ---------------------------------------------------------------------------
// formatBytesForMessage
// ---------------------------------------------------------------------------
describe('formatBytesForMessage', () => {
  it('formats zero bytes', () => {
    expect(formatBytesForMessage(0)).toBe('0 MB');
  });

  it('formats bytes in MB range', () => {
    const oneMB = 1024 * 1024;
    expect(formatBytesForMessage(oneMB)).toBe('1.00 MB');
    expect(formatBytesForMessage(oneMB * 5.5)).toBe('5.50 MB');
    expect(formatBytesForMessage(oneMB * 100)).toBe('100.00 MB');
  });

  it('formats bytes in GB range', () => {
    const oneGB = 1024 * 1024 * 1024;
    expect(formatBytesForMessage(oneGB)).toBe('1.00 GB');
    expect(formatBytesForMessage(oneGB * 2.5)).toBe('2.50 GB');
  });

  it('formats sub-megabyte values in MB', () => {
    expect(formatBytesForMessage(512 * 1024)).toBe('0.50 MB');
    expect(formatBytesForMessage(1024)).toBe('0.00 MB');
  });

  it('treats negative values as zero', () => {
    expect(formatBytesForMessage(-100)).toBe('0 MB');
    expect(formatBytesForMessage(-1024 * 1024)).toBe('0 MB');
  });

  it('treats NaN and Infinity as zero', () => {
    expect(formatBytesForMessage(NaN)).toBe('0 MB');
    expect(formatBytesForMessage(Infinity)).toBe('0 MB');
    expect(formatBytesForMessage(-Infinity)).toBe('0 MB');
  });

  it('formats the exact GB boundary', () => {
    const oneGB = 1024 * 1024 * 1024;
    expect(formatBytesForMessage(oneGB)).toBe('1.00 GB');
    expect(formatBytesForMessage(oneGB - 1)).toMatch(/MB$/);
  });
});
