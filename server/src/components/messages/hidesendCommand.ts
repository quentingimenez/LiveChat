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

const getTweetCardDurationSec = (
  medias: Array<{
    durationSec: number | null;
  }>,
) => {
  const knownDurations = medias
    .map((media) =>
      typeof media.durationSec === 'number' && Number.isFinite(media.durationSec) ? media.durationSec : 0,
    )
    .filter((durationSec) => durationSec > 0);

  if (knownDurations.length === 0) {
    return 15;
  }

  return Math.max(1, Math.ceil(Math.max(...knownDurations)));
};

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
        const tweetVideoMedias = await resolveTweetVideoMediasFromUrl(url || text);
        const tweetVideoMedia = tweetVideoMedias[0] || null;
        const tweetVideosForOverlay = tweetVideoMedias.slice(0, 2).map((video) => ({
          url: video.url,
          mime: video.mime,
          isVertical: video.isVertical,
          sourceStatusId: video.sourceStatusId,
          durationSec: video.durationSec,
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
              currentStatusId: currentTweetStatusId,
              videoUrl: tweetVideoMedia?.url || null,
              videoMime: tweetVideoMedia?.mime || null,
              videoIsVertical: tweetVideoMedia?.isVertical ?? null,
              videos: tweetVideosForOverlay,
            },
            caption: text || null,
          }),
          showText: false,
          durationSec: getTweetCardDurationSec(tweetVideoMedias),
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
