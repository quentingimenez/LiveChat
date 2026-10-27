import { describe, it, expect } from 'vitest';
import { getDiscordAvatarUrl, getDiscordDisplayName } from './discordUserSync';
import type { APIUser } from 'discord.js';

const makeUser = (overrides: Partial<APIUser> = {}): APIUser => ({
  id: '123456789',
  username: 'testuser',
  discriminator: '0',
  avatar: null,
  global_name: null,
  bot: false,
  system: false,
  mfa_enabled: false,
  banner: null,
  accent_color: null,
  locale: 'en-US',
  verified: false,
  flags: 0,
  premium_type: 0,
  public_flags: 0,
  ...overrides,
} as APIUser);

// ---------------------------------------------------------------------------
// getDiscordAvatarUrl
// ---------------------------------------------------------------------------
describe('getDiscordAvatarUrl', () => {
  it('returns null when user has no avatar', () => {
    const user = makeUser({ avatar: null });
    expect(getDiscordAvatarUrl(user)).toBeNull();
  });

  it('returns png URL for static avatar', () => {
    const user = makeUser({ id: '111', avatar: 'abc123' });
    expect(getDiscordAvatarUrl(user)).toBe(
      'https://cdn.discordapp.com/avatars/111/abc123.png?size=256',
    );
  });

  it('returns gif URL for animated avatar (a_ prefix)', () => {
    const user = makeUser({ id: '222', avatar: 'a_animated' });
    expect(getDiscordAvatarUrl(user)).toBe(
      'https://cdn.discordapp.com/avatars/222/a_animated.gif?size=256',
    );
  });
});

// ---------------------------------------------------------------------------
// getDiscordDisplayName
// ---------------------------------------------------------------------------
describe('getDiscordDisplayName', () => {
  it('returns global_name when available', () => {
    const user = makeUser({ global_name: 'DisplayName', username: 'user123' });
    expect(getDiscordDisplayName(user)).toBe('DisplayName');
  });

  it('falls back to username when global_name is null', () => {
    const user = makeUser({ global_name: null, username: 'user123' });
    expect(getDiscordDisplayName(user)).toBe('user123');
  });

  it('falls back to username when global_name is empty string', () => {
    const user = makeUser({ global_name: '' as any, username: 'user123' });
    expect(getDiscordDisplayName(user)).toBe('user123');
  });
});
