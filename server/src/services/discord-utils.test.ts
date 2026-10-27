import { describe, it, expect, vi } from 'vitest';
import { resolveDiscordAuthorName, resolveDiscordAuthorImage } from './discord-utils';

const makeInteraction = (overrides: {
  nick?: string | null;
  globalName?: string | null;
  username?: string;
  guildId?: string | null;
  userId?: string;
  memberAvatar?: string | null;
  displayAvatarURL?: string;
} = {}) => {
  return {
    member: overrides.nick !== undefined ? { nick: overrides.nick } : null,
    user: {
      id: overrides.userId ?? '123456',
      globalName: overrides.globalName ?? null,
      username: overrides.username ?? 'fallback_username',
      displayAvatarURL: vi.fn().mockReturnValue(
        overrides.displayAvatarURL ?? 'https://cdn.discordapp.com/embed/avatars/0.png',
      ),
    },
    guildId: overrides.guildId ?? null,
  } as any;
};

// ---------------------------------------------------------------------------
// resolveDiscordAuthorName
// ---------------------------------------------------------------------------
describe('resolveDiscordAuthorName', () => {
  it('prefers guild member nickname', () => {
    const interaction = makeInteraction({ nick: 'NickName', globalName: 'Global', username: 'user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('NickName');
  });

  it('falls back to global name when no nickname', () => {
    const interaction = makeInteraction({ nick: null, globalName: 'GlobalDisplay', username: 'user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('GlobalDisplay');
  });

  it('falls back to username when no nickname or global name', () => {
    const interaction = makeInteraction({ nick: null, globalName: null, username: 'raw_user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('raw_user');
  });

  it('ignores empty/whitespace nickname', () => {
    const interaction = makeInteraction({ nick: '   ', globalName: 'Global', username: 'user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('Global');
  });

  it('ignores empty/whitespace global name', () => {
    const interaction = makeInteraction({ nick: null, globalName: '  ', username: 'user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('user');
  });

  it('handles no member at all', () => {
    const interaction = makeInteraction({ globalName: null, username: 'solo_user' });
    expect(resolveDiscordAuthorName(interaction)).toBe('solo_user');
  });
});

// ---------------------------------------------------------------------------
// resolveDiscordAuthorImage
// ---------------------------------------------------------------------------
describe('resolveDiscordAuthorImage', () => {
  it('returns guild-specific avatar URL when guild and member avatar exist', () => {
    const interaction = makeInteraction({
      guildId: 'guild_1',
      userId: 'user_1',
      nick: 'SomeNick', // ensures member is not null
      memberAvatar: 'abc123',
    });
    (interaction.member as any).avatar = 'abc123';

    const url = resolveDiscordAuthorImage(interaction);
    expect(url).toBe(
      'https://cdn.discordapp.com/guilds/guild_1/users/user_1/avatars/abc123.png?size=256',
    );
  });

  it('uses gif extension for animated avatars (a_ prefix)', () => {
    const interaction = makeInteraction({
      guildId: 'guild_1',
      userId: 'user_1',
    });
    (interaction.member as any) = { avatar: 'a_animated_hash' };

    const url = resolveDiscordAuthorImage(interaction);
    expect(url).toContain('.gif?size=256');
  });

  it('falls back to displayAvatarURL when no guild avatar', () => {
    const fallbackUrl = 'https://cdn.discordapp.com/avatars/123/hash.png';
    const interaction = makeInteraction({
      guildId: 'guild_1',
      displayAvatarURL: fallbackUrl,
    });

    const url = resolveDiscordAuthorImage(interaction);
    expect(url).toBe(fallbackUrl);
    expect(interaction.user.displayAvatarURL).toHaveBeenCalledWith({
      extension: 'png',
      size: 256,
    });
  });

  it('falls back to displayAvatarURL when no guildId', () => {
    const interaction = makeInteraction({ guildId: null });
    (interaction.member as any) = { avatar: 'some_hash' };

    const url = resolveDiscordAuthorImage(interaction);
    // No guildId => fallback path
    expect(url).toBe('https://cdn.discordapp.com/embed/avatars/0.png');
  });
});
