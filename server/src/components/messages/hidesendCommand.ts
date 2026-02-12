import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';
import { encodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { resolveTweetCardFromUrl, resolveTweetCardFromUrlWithOptions } from '../../services/social/twitterOEmbed';
import { resolveTweetVideoMediaFromUrl } from '../../services/social/twitterVideoResolver';

export const hideSendCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('hideSendCommand')!)
    .setDescription(rosetty.t('hideSendCommandDescription')!)
    .addStringOption((option) =>
      option
        .setName(rosetty.t('hideSendCommandOptionURL')!)
        .setDescription(rosetty.t('hideSendCommandOptionURLDescription')!),
    )
    .addAttachmentOption((option) =>
      option
        .setName(rosetty.t('hideSendCommandOptionMedia')!)
        .setDescription(rosetty.t('hideSendCommandOptionMediaDescription')!),
    )
    .addStringOption((option) =>
      option
        .setName(rosetty.t('hideSendCommandOptionText')!)
        .setDescription(rosetty.t('hideSendCommandOptionTextDescription')!)
        .setRequired(false),
    ),
  handler: async (interaction: CommandInteraction) => {
    const url = interaction.options.get(rosetty.t('hideSendCommandOptionURL')!)?.value as string | null;
    const text = interaction.options.get(rosetty.t('hideSendCommandOptionText')!)?.value as string | null;
    const attachment = interaction.options.get(rosetty.t('hideSendCommandOptionMedia')!)?.attachment;
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

    await interaction.deferReply({
      ephemeral: true,
    });

    try {
      const tweetCard = !media ? await resolveTweetCardFromUrl(url || text) : null;

      if (tweetCard) {
        const tweetVideoMedia = await resolveTweetVideoMediaFromUrl(url || text);
        const tweetCardForOverlay =
          tweetVideoMedia && !media
            ? (await resolveTweetCardFromUrlWithOptions(url || text, {
                hideMedia: true,
              })) || tweetCard
            : tweetCard;

        await createPlaybackJob({
          guildId: interaction.guildId!,
          text: encodeRichOverlayPayload({
            type: 'tweet',
            tweetCard: {
              ...tweetCardForOverlay,
              videoUrl: tweetVideoMedia?.url || null,
              videoMime: tweetVideoMedia?.mime || null,
              videoIsVertical: tweetVideoMedia?.isVertical ?? null,
            },
            caption: text || null,
          }),
          showText: false,
          durationSec: env.TWITTER_CARD_DURATION_SEC,
        });

        await interaction.editReply({
          embeds: [
            new EmbedBuilder()
              .setTitle(rosetty.t('success')!)
              .setDescription(rosetty.t('hideSendCommandAnswer')!)
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
        });

        await interaction.editReply({
          embeds: [
            new EmbedBuilder()
              .setTitle(rosetty.t('success')!)
              .setDescription(rosetty.t('hideSendCommandAnswer')!)
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
      });

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('success')!)
            .setDescription(rosetty.t('hideSendCommandAnswer')!)
            .setColor(0x2ecc71),
        ],
      });
    } catch (error) {
      const mediaError = toMediaIngestionError(error);
      logger.error(mediaError, `[MEDIA] hide send command failed (${mediaError.code})`);

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
