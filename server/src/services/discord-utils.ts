import { CommandInteraction } from 'discord.js';
import { toNonEmptyString } from './stringUtils';

export const resolveDiscordAuthorName = (interaction: CommandInteraction): string => {
  const memberNick = toNonEmptyString((interaction.member as { nick?: unknown } | null)?.nick);
  if (memberNick) {
    return memberNick;
  }

  const globalName = toNonEmptyString(interaction.user.globalName);
  if (globalName) {
    return globalName;
  }

  return interaction.user.username;
};

export const resolveDiscordAuthorImage = (interaction: CommandInteraction): string => {
  const memberAvatarHash = toNonEmptyString((interaction.member as { avatar?: unknown } | null)?.avatar);

  if (interaction.guildId && memberAvatarHash) {
    const extension = memberAvatarHash.startsWith('a_') ? 'gif' : 'png';
    return `https://cdn.discordapp.com/guilds/${interaction.guildId}/users/${interaction.user.id}/avatars/${memberAvatarHash}.${extension}?size=256`;
  }

  return interaction.user.displayAvatarURL({
    extension: 'png',
    size: 256,
  });
};
