import { describe, expect, it } from 'vitest';
import { parseDisplayDuration } from './duration';

describe('parseDisplayDuration', () => {
  it('treats an empty field as no duration', () => {
    expect(parseDisplayDuration('')).toEqual({ ok: true, value: null });
    expect(parseDisplayDuration('  ')).toEqual({ ok: true, value: null });
  });

  it('accepts whole seconds from 1 to 600', () => {
    expect(parseDisplayDuration('8')).toEqual({ ok: true, value: 8 });
    expect(parseDisplayDuration(' 600 ')).toEqual({ ok: true, value: 600 });
    expect(parseDisplayDuration('1')).toEqual({ ok: true, value: 1 });
  });

  it('rejects out of range, fractional and non numeric values', () => {
    for (const raw of ['0', '601', '8.5', 'abc', '-3', '1e2']) {
      expect(parseDisplayDuration(raw)).toEqual({ ok: false });
    }
  });
});
