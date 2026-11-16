import { describe, expect, it } from 'vitest';
import {
  ago,
  bytes,
  codeDisplay,
  duration,
  expiresIn,
  initials,
  percent,
  plural,
  uptime,
  versionBelow,
} from './format';

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const NOW = Date.parse('2026-11-15T12:00:00.000Z');

describe('format', () => {
  it('formats bytes in Mo and Go with a decimal comma', () => {
    expect(bytes(0)).toBe('0 Mo');
    expect(bytes(512 * MIB)).toBe('512 Mo');
    expect(bytes(9.4 * GIB)).toBe('9,4 Go');
    expect(bytes(-5)).toBe('0 Mo');
  });

  it('pluralises in French', () => {
    expect(plural(1, 'média', 'médias')).toBe('1 média');
    expect(plural(2, 'média', 'médias')).toBe('2 médias');
    expect(plural(0, 'entrée', 'entrées')).toBe('0 entrée');
  });

  it('formats relative times from ISO dates', () => {
    expect(ago(null, NOW)).toBe('jamais');
    expect(ago('2026-11-15T11:59:50.000Z', NOW)).toBe('à l’instant');
    expect(ago('2026-11-15T11:55:00.000Z', NOW)).toBe('il y a 5 min');
    expect(ago('2026-11-15T10:00:00.000Z', NOW)).toBe('il y a 2 h');
    expect(ago('2026-11-12T12:00:00.000Z', NOW)).toBe('il y a 3 j');
  });

  it('formats expiry delays', () => {
    expect(expiresIn(null, NOW)).toBe('jamais');
    expect(expiresIn('2026-11-15T11:30:00.000Z', NOW)).toBe('expiré');
    expect(expiresIn('2026-11-15T12:30:00.000Z', NOW)).toBe('dans 30 min');
    expect(expiresIn('2026-11-17T12:00:00.000Z', NOW)).toBe('dans 2 j');
  });

  it('builds initials from accents and spaces', () => {
    expect(initials('Quentin')).toBe('Q');
    expect(initials('Élise Martin')).toBe('ÉM');
    expect(initials('')).toBe('');
  });

  it('compares overlay versions', () => {
    expect(versionBelow('3.1.2', '3.2.0')).toBe(true);
    expect(versionBelow('3.2', '3.2.0')).toBe(false);
    expect(versionBelow('3.4.0', '3.2.0')).toBe(false);
    expect(versionBelow(null, '3.2.0')).toBe(true);
  });

  it('clamps percentages and formats durations and uptime', () => {
    expect(percent(5, 10)).toBe(50);
    expect(percent(-1, 10)).toBe(0);
    expect(percent(20, 10)).toBe(100);
    expect(percent(1, 0)).toBe(0);
    expect(duration(14)).toBe('14 s');
    expect(duration(90)).toBe('1 min 30 s');
    expect(uptime(6 * 86400 + 4 * 3600)).toBe('6 j 4 h');
    expect(codeDisplay('ABCD2345')).toBe('ABCD-2345');
  });
});
