import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { addToMemeBoard } from '../../services/memeBoard';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';

const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

const resolveAuthorName = (interaction: CommandInteraction): string => {
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

export const memeAddCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('memeAddCommand')!)
    .setDescription(rosetty.t('memeAddCommandDescription')!)
    .addStringOption((option) =>
      option
        .setName(rosetty.t('memeAddCommandOptionURL')!)
        .setDescription(rosetty.t('memeAddCommandOptionURLDescription')!)
        .setRequired(false),
    )
    .addAttachmentOption((option) =>
      option
        .setName(rosetty.t('memeAddCommandOptionMedia')!)
        .setDescription(rosetty.t('memeAddCommandOptionMediaDescription')!)
        .setRequired(false),
    )
    .addStringOption((option) =>
      option
        .setName(rosetty.t('memeAddCommandOptionTitle')!)
        .setDescription(rosetty.t('memeAddCommandOptionTitleDescription')!)
        .setRequired(false),
    )
    .addBooleanOption((option) =>
      option
        .setName(rosetty.t('memeAddCommandOptionForceRefresh')!)
        .setDescription(rosetty.t('memeAddCommandOptionForceRefreshDescription')!)
        .setRequired(false),
    ),
  handler: async (interaction: CommandInteraction) => {
    const guildId = interaction.guildId;
    const url = interaction.options.get(rosetty.t('memeAddCommandOptionURL')!)?.value as string | null;
    const title = interaction.options.get(rosetty.t('memeAddCommandOptionTitle')!)?.value as string | null;
    const attachment = interaction.options.get(rosetty.t('memeAddCommandOptionMedia')!)?.attachment;
    const forceRefresh = interaction.options.get(rosetty.t('memeAddCommandOptionForceRefresh')!)?.value === true;
    const media = attachment?.url || attachment?.proxyURL;

    if (!guildId) {
      await interaction.reply({
        embeds: [
          new EmbedBuilder().setTitle(rosetty.t('error')!).setDescription(rosetty.t('notAllowed')!).setColor(0xe74c3c),
        ],
        ephemeral: true,
      });
      return;
    }

    if (!url && !media) {
      await interaction.reply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('error')!)
            .setDescription(rosetty.t('memeAddCommandMissingMedia')!)
            .setColor(0xe74c3c),
        ],
        ephemeral: true,
      });
      return;
    }

    await interaction.deferReply({
      ephemeral: true,
    });

    try {
      const mediaAsset = await ingestMediaFromSource({
        url,
        media,
        forceRefresh,
      });

      if (!mediaAsset) {
        await interaction.editReply({
          embeds: [
            new EmbedBuilder()
              .setTitle(rosetty.t('error')!)
              .setDescription(rosetty.t('memeAddCommandMissingMedia')!)
              .setColor(0xe74c3c),
          ],
        });
        return;
      }

      const addResult = await addToMemeBoard({
        guildId,
        mediaAssetId: mediaAsset.id,
        title,
        createdByDiscordUserId: interaction.user.id,
        createdByName: resolveAuthorName(interaction),
      });

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('success')!)
            .setDescription(
              addResult.created ? rosetty.t('memeAddCommandAnswerCreated')! : rosetty.t('memeAddCommandAnswerExists')!,
            )
            .setColor(0x2ecc71),
        ],
      });
    } catch (error) {
      const mediaError = toMediaIngestionError(error);
      logger.error(
        {
          err: mediaError,
          sourceUrl: url || null,
          sourceMedia: media || null,
        },
        `[MEDIA] meme-add command failed (${mediaError.code})`,
      );

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('error')!)
            .setDescription(getLocalizedMediaErrorMessage(mediaError))
            .setColor(0xe74c3c),
        ],
      });
    }
  },
});
