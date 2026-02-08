import crypto from 'crypto';
import { addMinutes } from 'date-fns';
import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';

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

export const overlayCodeCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('overlayCodeCommand')!)
    .setDescription(rosetty.t('overlayCodeCommandDescription')!),
  handler: async (interaction: CommandInteraction) => {
    const code = await generateUniquePairingCode();
    const expiresAt = addMinutes(new Date(), Math.max(1, env.PAIRING_CODE_TTL_MINUTES));

    await prisma.pairingCode.create({
      data: {
        code,
        guildId: interaction.guildId!,
        createdByDiscordUserId: interaction.user.id,
        expiresAt,
      },
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
