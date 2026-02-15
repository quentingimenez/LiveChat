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
      const tweetInput = url || text;
      let tweetVideoMedias: Awaited<ReturnType<typeof resolveTweetVideoMediasFromUrl>> = [];
      let tweetCard = null;
      const tweetResolveStartedAt = Date.now();

      if (!media && tweetInput) {
        const [videoResult, cardResult] = await Promise.allSettled([
          resolveTweetVideoMediasFromUrl(tweetInput),
          resolveTweetCardFromUrl(tweetInput),
        ]);

        if (videoResult.status === 'fulfilled') {
          tweetVideoMedias = videoResult.value;
        } else {
          logger.warn(
            {
              err: toMediaIngestionError(videoResult.reason),
              sourceUrl: tweetInput,
            },
            '[MEDIA] Tweet video resolution failed, continuing with fallback',
          );
        }

        if (cardResult.status === 'fulfilled') {
          tweetCard = cardResult.value;
        } else {
          logger.warn(
            {
              err: toMediaIngestionError(cardResult.reason),
              sourceUrl: tweetInput,
            },
            '[MEDIA] Tweet card resolution failed, continuing with media-only playback',
          );
        }

        logger.info(
          {
            sourceUrl: tweetInput,
            elapsedMs: Date.now() - tweetResolveStartedAt,
            videoCandidates: tweetVideoMedias.length,
            hasTweetCard: !!tweetCard,
          },
          '[MEDIA] Tweet metadata resolved',
        );
      }

      const tweetVideoMedia = tweetVideoMedias[0] || null;

      if (tweetCard) {
        const tweetVideosForOverlay = tweetVideoMedias.slice(0, 2).map((video) => ({
          url: video.url,
          mime: video.mime,
          isVertical: video.isVertical,
          sourceStatusId: video.sourceStatusId,
          durationSec: video.durationSec,
        }));
        const currentTweetStatusId = extractTweetStatusIdFromUrl(tweetInput);
        const hasCurrentTweetVideo =
          !!currentTweetStatusId &&
          tweetVideosForOverlay.some((video) => video.sourceStatusId === currentTweetStatusId);
        const hasOtherTweetVideo =
          !!currentTweetStatusId &&
          tweetVideosForOverlay.some(
            (video) => !!video.sourceStatusId && video.sourceStatusId !== currentTweetStatusId,
          );
        const shouldHideCardMedia = !media && !!currentTweetStatusId && hasCurrentTweetVideo && hasOtherTweetVideo;
        let tweetCardForOverlay = tweetCard;

        if (shouldHideCardMedia) {
          try {
            tweetCardForOverlay =
              (await resolveTweetCardFromUrlWithOptions(tweetInput, {
                hideMedia: true,
              })) || tweetCard;
          } catch (error) {
            logger.warn(
              {
                err: toMediaIngestionError(error),
                sourceUrl: tweetInput || null,
              },
              '[MEDIA] Tweet card hide-media resolution failed, keeping original card',
            );
          }
        }

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
          authorName: null,
          authorImage: null,
          durationSec: getTweetCardDurationSec(tweetVideoMedias),
          source: 'discord_send_command',
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
          source: 'discord_send_command',
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
        source: 'discord_send_command',
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
      logger.error(
        {
          err: mediaError,
          sourceUrl: url || null,
          sourceMedia: media || null,
        },
        `[MEDIA] send command failed (${mediaError.code})`,
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
