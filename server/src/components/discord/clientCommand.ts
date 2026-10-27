import crypto from 'crypto';
import { addMinutes } from 'date-fns';
import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { toNonEmptyString } from '../../services/stringUtils';
import { resolveDiscordAuthorName, resolveDiscordAuthorImage } from '../../services/discord-utils';

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


const PAIRING_MODE_NORMAL = 'normal';
const PAIRING_MODE_INVITE = 'invite';

export const overlayCodeCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('overlayCodeCommand')!)
    .setDescription(rosetty.t('overlayCodeCommandDescription')!)
    .addStringOption((option) =>
      option
        .setName(rosetty.t('overlayCodeCommandOptionMode')!)
        .setDescription(rosetty.t('overlayCodeCommandOptionModeDescription')!)
        .setRequired(false)
        .addChoices(
          {
            name: rosetty.t('overlayCodeCommandOptionModeChoiceNormal')!,
            value: PAIRING_MODE_NORMAL,
          },
          {
            name: rosetty.t('overlayCodeCommandOptionModeChoiceInvite')!,
            value: PAIRING_MODE_INVITE,
          },
        ),
    ),
  handler: async (interaction: CommandInteraction) => {
    const selectedModeRaw = interaction.options.get(rosetty.t('overlayCodeCommandOptionMode')!)?.value;
    const selectedMode = selectedModeRaw === PAIRING_MODE_INVITE ? PAIRING_MODE_INVITE : PAIRING_MODE_NORMAL;
    const isInviteMode = selectedMode === PAIRING_MODE_INVITE;
    const code = await generateUniquePairingCode();
    const expiresAt = isInviteMode ? null : addMinutes(new Date(), Math.max(1, env.PAIRING_CODE_TTL_MINUTES));
    const authorName = resolveDiscordAuthorName(interaction);
    const authorImage = resolveDiscordAuthorImage(interaction);
    const pairingCodeMode = isInviteMode ? 'INVITE_READ_ONLY' : 'NORMAL';

    await prisma.pairingCode.create({
      data: {
        code,
        guildId: interaction.guildId!,
        mode: pairingCodeMode,
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
            isInviteMode
              ? rosetty.t('overlayCodeCommandAnswerDescriptionInvite', {
                  code,
                  apiUrl: env.API_URL,
                })!
              : rosetty.t('overlayCodeCommandAnswerDescription', {
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
