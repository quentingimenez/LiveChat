import crypto from 'crypto';
import { addMinutes } from 'date-fns';
import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized || null;
};

const randomPairingCode = () => {
  const charset = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const bytes = crypto.randomBytes(6);

  let code = '';

  for (const byte of bytes) {
    code += charset[byte % charset.length];
  }

  return code;
};

const generateUniquePairingCode = async () => {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    const candidate = randomPairingCode();
    const exists = await prisma.pairingCode.findUnique({
      where: {
        code: candidate,
      },
    });

    if (!exists) {
      return candidate;
    }
  }

  throw new Error('unable_to_generate_pairing_code');
};

const resolvePairingAuthorName = (interaction: CommandInteraction): string => {
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

const resolvePairingAuthorImage = (interaction: CommandInteraction): string => {
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

export const overlayCodeCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('overlayCodeCommand')!)
    .setDescription(rosetty.t('overlayCodeCommandDescription')!),
  handler: async (interaction: CommandInteraction) => {
    const code = await generateUniquePairingCode();
    const expiresAt = addMinutes(new Date(), Math.max(1, env.PAIRING_CODE_TTL_MINUTES));
    const authorName = resolvePairingAuthorName(interaction);
    const authorImage = resolvePairingAuthorImage(interaction);

    await prisma.pairingCode.create({
      data: {
        code,
        guildId: interaction.guildId!,
        createdByDiscordUserId: interaction.user.id,
        authorName,
        authorImage,
        expiresAt,
      } as any,
    });

    await interaction.reply({
      embeds: [
        new EmbedBuilder()
          .setTitle(rosetty.t('overlayCodeCommandAnswerTitle')!)
          .setDescription(
            rosetty.t('overlayCodeCommandAnswerDescription', {
              code,
              expiresIn: env.PAIRING_CODE_TTL_MINUTES,
              apiUrl: env.API_URL,
            })!,
          )
          .setColor(0x3498db),
      ],
      ephemeral: true,
    });
  },
});
