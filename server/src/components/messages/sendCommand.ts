import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { ingestMediaFromSource } from '../../services/media/mediaIngestion';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';
import { encodeRichOverlayPayload } from '../../services/messages/richOverlayPayload';
import { resolveTweetCardFromUrl, resolveTweetCardFromUrlWithOptions } from '../../services/social/twitterOEmbed';
import {
  extractTweetStatusIdFromUrl,
  resolveTweetVideoMediasFromUrl,
} from '../../services/social/twitterVideoResolver';

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
        const tweetVideoMedias = await resolveTweetVideoMediasFromUrl(url || text);
        const tweetVideoMedia = tweetVideoMedias[0] || null;
        const tweetVideosForOverlay = tweetVideoMedias.slice(0, 2).map((video) => ({
          url: video.url,
          mime: video.mime,
          isVertical: video.isVertical,
          sourceStatusId: video.sourceStatusId,
        }));
        const currentTweetStatusId = extractTweetStatusIdFromUrl(url || text);
        const shouldHideCardMedia =
          !media &&
          !!currentTweetStatusId &&
          tweetVideosForOverlay.some((video) => video.sourceStatusId === currentTweetStatusId);
        const tweetCardForOverlay = shouldHideCardMedia
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
              videos: tweetVideosForOverlay,
            },
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

      const tweetVideoMedias = !media ? await resolveTweetVideoMediasFromUrl(url || text) : [];
      const tweetVideoMedia = tweetVideoMedias[0] || null;

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
