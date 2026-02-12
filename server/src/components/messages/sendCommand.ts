import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';
import { encodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { resolveTweetCardFromUrl } from '../../services/social/twitterOEmbed';
import { resolveTweetVideoMediaFromUrl } from '../../services/social/twitterVideoResolver';

export const sendCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('sendCommand')!)
    .setDescription(rosetty.t('sendCommandDescription')!)
    .addStringOption((option) =>
      option.setName(rosetty.t('sendCommandOptionURL')!).setDescription(rosetty.t('sendCommandOptionURLDescription')!),
    )
    .addAttachmentOption((option) =>
      option
        .setName(rosetty.t('sendCommandOptionMedia')!)
        .setDescription(rosetty.t('sendCommandOptionMediaDescription')!),
    )
    .addStringOption((option) =>
      option
        .setName(rosetty.t('sendCommandOptionText')!)
        .setDescription(rosetty.t('sendCommandOptionTextDescription')!)
        .setRequired(false),
    ),
  handler: async (interaction: CommandInteraction) => {
    const url = interaction.options.get(rosetty.t('sendCommandOptionURL')!)?.value as string | null;
    const text = interaction.options.get(rosetty.t('sendCommandOptionText')!)?.value as string | null;
    const attachment = interaction.options.get(rosetty.t('sendCommandOptionMedia')!)?.attachment;
    const media = attachment?.url || attachment?.proxyURL;

    if (!url && !media && !text) {
      await interaction.reply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('error')!)
            .setDescription(rosetty.t('sendCommandMissingContent')!)
            .setColor(0xe74c3c),
        ],
        ephemeral: true,
      });
      return;
    }

    await interaction.deferReply();

    try {
      const tweetCard = !media ? await resolveTweetCardFromUrl(url || text) : null;

      if (tweetCard) {
        await createPlaybackJob({
          guildId: interaction.guildId!,
          text: encodeRichOverlayPayload({
            type: 'tweet',
            tweetCard,
            caption: text || null,
          }),
          showText: false,
          authorName: null,
          authorImage: null,
          durationSec: env.TWITTER_CARD_DURATION_SEC,
        });

        await interaction.editReply({
          embeds: [
            new EmbedBuilder()
              .setTitle(rosetty.t('success')!)
              .setDescription(rosetty.t('sendCommandAnswer')!)
              .setColor(0x2ecc71),
          ],
        });
        return;
      }

      const tweetVideoMedia = !media ? await resolveTweetVideoMediaFromUrl(url || text) : null;

      if (tweetVideoMedia) {
        const mediaAsset = await ingestMediaFromSource({
          media: tweetVideoMedia.url,
        });

        await createPlaybackJob({
          guildId: interaction.guildId!,
          mediaAsset,
          text,
          showText: !!text,
          authorName: interaction.user.username,
          authorImage: interaction.user.avatarURL(),
        });

        await interaction.editReply({
          embeds: [
            new EmbedBuilder()
              .setTitle(rosetty.t('success')!)
              .setDescription(rosetty.t('sendCommandAnswer')!)
              .setColor(0x2ecc71),
          ],
        });
        return;
      }

      let mediaAsset = null;

      if (url || media) {
        mediaAsset = await ingestMediaFromSource({
          url,
          media,
        });
      }

      await createPlaybackJob({
        guildId: interaction.guildId!,
        mediaAsset,
        text,
        showText: !!text,
        authorName: interaction.user.username,
        authorImage: interaction.user.avatarURL(),
      });

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('success')!)
            .setDescription(rosetty.t('sendCommandAnswer')!)
            .setColor(0x2ecc71),
        ],
      });
    } catch (error) {
      const mediaError = toMediaIngestionError(error);
      logger.error(mediaError, `[MEDIA] send command failed (${mediaError.code})`);

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
