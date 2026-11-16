import { describe, expect, it } from 'vitest';
import { buildHash, parseHash } from './router';

describe('hash router', () => {
  it('defaults to the dashboard on an empty hash', () => {
    expect(parseHash('')).toEqual({ name: 'dashboard', query: {} });
    expect(parseHash('#')).toEqual({ name: 'dashboard', query: {} });
  });

  it('parses top-level screens', () => {
    expect(parseHash('#/devices')).toEqual({ name: 'devices', query: {} });
    expect(parseHash('#/settings')).toEqual({ name: 'settings', query: {} });
    expect(parseHash('#/guilds')).toEqual({ name: 'guilds', query: {} });
  });

  it('parses a guild and its tab', () => {
    expect(parseHash('#/guilds/812345/settings')).toEqual({
      name: 'guild',
      guildId: '812345',
      tab: 'settings',
      query: {},
    });
  });

  it('falls back to the overview tab for an unknown tab', () => {
    expect(parseHash('#/guilds/812345/nope')).toMatchObject({
      name: 'guild',
      guildId: '812345',
      tab: 'overview',
    });
  });

  it('parses query parameters (OAuth errors)', () => {
    expect(parseHash('#/login?error=not_allowed')).toEqual({
      name: 'login',
      query: { error: 'not_allowed' },
    });
  });

  it('falls back to the dashboard for unknown screens', () => {
    expect(parseHash('#/does-not-exist')).toEqual({ name: 'dashboard', query: {} });
  });

  it('builds guild and login hashes', () => {
    expect(
      buildHash({ name: 'guild', guildId: '812345', tab: 'members', query: {} }),
    ).toBe('#/guilds/812345/members');
    expect(buildHash({ name: 'guilds', query: {} })).toBe('#/guilds');
    expect(buildHash({ name: 'login', query: { error: 'oauth_failed' } })).toBe(
      '#/login?error=oauth_failed',
    );
  });

  it('round-trips a parsed route', () => {
    const route = parseHash('#/guilds/42/queue');
    expect(parseHash(buildHash(route))).toEqual(route);
  });
});
