import { describe, expect, it } from 'vitest';
import { isNonEmptyString, toNonEmptyString, isRecord } from './utils';

describe('isNonEmptyString', () => {
  it('returns true for non-empty strings', () => {
    expect(isNonEmptyString('hello')).toBe(true);
    expect(isNonEmptyString('a')).toBe(true);
    expect(isNonEmptyString('  content  ')).toBe(true);
  });

  it('returns false for empty or whitespace-only strings', () => {
    expect(isNonEmptyString('')).toBe(false);
    expect(isNonEmptyString('   ')).toBe(false);
    expect(isNonEmptyString('\t\n')).toBe(false);
  });

  it('returns false for non-string types', () => {
    expect(isNonEmptyString(null)).toBe(false);
    expect(isNonEmptyString(undefined)).toBe(false);
    expect(isNonEmptyString(0)).toBe(false);
    expect(isNonEmptyString(42)).toBe(false);
    expect(isNonEmptyString(true)).toBe(false);
    expect(isNonEmptyString(false)).toBe(false);
    expect(isNonEmptyString({})).toBe(false);
    expect(isNonEmptyString([])).toBe(false);
  });
});

describe('toNonEmptyString', () => {
  it('returns trimmed string for non-empty input', () => {
    expect(toNonEmptyString('hello')).toBe('hello');
    expect(toNonEmptyString('  hello  ')).toBe('hello');
    expect(toNonEmptyString('\thello\n')).toBe('hello');
  });

  it('returns null for empty or whitespace-only strings', () => {
    expect(toNonEmptyString('')).toBeNull();
    expect(toNonEmptyString('   ')).toBeNull();
    expect(toNonEmptyString('\t\n')).toBeNull();
  });

  it('returns null for non-string types', () => {
    expect(toNonEmptyString(null)).toBeNull();
    expect(toNonEmptyString(undefined)).toBeNull();
    expect(toNonEmptyString(0)).toBeNull();
    expect(toNonEmptyString(42)).toBeNull();
    expect(toNonEmptyString(true)).toBeNull();
    expect(toNonEmptyString({})).toBeNull();
    expect(toNonEmptyString([])).toBeNull();
  });
});

describe('isRecord', () => {
  it('returns true for plain objects', () => {
    expect(isRecord({})).toBe(true);
    expect(isRecord({ key: 'value' })).toBe(true);
    expect(isRecord({ nested: { a: 1 } })).toBe(true);
  });

  it('returns false for null', () => {
    expect(isRecord(null)).toBe(false);
  });

  it('returns false for arrays', () => {
    expect(isRecord([])).toBe(false);
    expect(isRecord([1, 2, 3])).toBe(false);
  });

  it('returns false for primitives', () => {
    expect(isRecord(undefined)).toBe(false);
    expect(isRecord('')).toBe(false);
    expect(isRecord('hello')).toBe(false);
    expect(isRecord(42)).toBe(false);
    expect(isRecord(true)).toBe(false);
  });
});
