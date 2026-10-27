import { describe, it, expect } from 'vitest';
import { normalizeOverlaySessionMode, hashOverlayToken, generateOverlayToken } from './overlayAuth';

// ---------------------------------------------------------------------------
// normalizeOverlaySessionMode
// ---------------------------------------------------------------------------
describe('normalizeOverlaySessionMode', () => {
  it('returns "invite_read_only" for exact match', () => {
    expect(normalizeOverlaySessionMode('invite_read_only')).toBe('invite_read_only');
  });

  it('is case-insensitive', () => {
    expect(normalizeOverlaySessionMode('INVITE_READ_ONLY')).toBe('invite_read_only');
    expect(normalizeOverlaySessionMode('Invite_Read_Only')).toBe('invite_read_only');
  });

  it('trims whitespace', () => {
    expect(normalizeOverlaySessionMode('  invite_read_only  ')).toBe('invite_read_only');
  });

  it('returns "normal" for any other string', () => {
    expect(normalizeOverlaySessionMode('normal')).toBe('normal');
    expect(normalizeOverlaySessionMode('NORMAL')).toBe('normal');
    expect(normalizeOverlaySessionMode('')).toBe('normal');
    expect(normalizeOverlaySessionMode('other')).toBe('normal');
  });

  it('returns "normal" for non-string types', () => {
    expect(normalizeOverlaySessionMode(null)).toBe('normal');
    expect(normalizeOverlaySessionMode(undefined)).toBe('normal');
    expect(normalizeOverlaySessionMode(0)).toBe('normal');
    expect(normalizeOverlaySessionMode(true)).toBe('normal');
    expect(normalizeOverlaySessionMode({})).toBe('normal');
  });
});

// ---------------------------------------------------------------------------
// hashOverlayToken
// ---------------------------------------------------------------------------
describe('hashOverlayToken', () => {
  it('returns a 64 character hex string', () => {
    const hash = hashOverlayToken('test-token');
    expect(hash).toMatch(/^[a-f0-9]{64}$/);
  });

  it('produces consistent hashes for same input', () => {
    const a = hashOverlayToken('my-token');
    const b = hashOverlayToken('my-token');
    expect(a).toBe(b);
  });

  it('produces different hashes for different input', () => {
    const a = hashOverlayToken('token-a');
    const b = hashOverlayToken('token-b');
    expect(a).not.toBe(b);
  });
});

// ---------------------------------------------------------------------------
// generateOverlayToken
// ---------------------------------------------------------------------------
describe('generateOverlayToken', () => {
  it('returns a 64 character hex string', () => {
    const token = generateOverlayToken();
    expect(token).toMatch(/^[a-f0-9]{64}$/);
  });

  it('generates unique tokens', () => {
    const tokens = new Set(Array.from({ length: 20 }, () => generateOverlayToken()));
    expect(tokens.size).toBe(20);
  });
});
