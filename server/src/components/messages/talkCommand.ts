import crypto from 'crypto';
import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { deleteGtts, promisedGtts } from '../../services/gtts';
import { ingestMediaFromLocalFile } from '../../services/media/mediaIngestion';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';

export const talkCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('talkCommand')!)
    .setDescription(rosetty.t('talkCommandDescription')!)
    .addStringOption((option) =>
      option
        .setName(rosetty.t('talkCommandOptionVoice')!)
        .setDescription(rosetty.t('talkCommandOptionVoiceDescription')!)
        .setRequired(true),
    )
    .addStringOption((option) =>
      option
        .setName(rosetty.t('talkCommandOptionText')!)
        .setDescription(rosetty.t('talkCommandOptionTextDescription')!),
    ),
  handler: async (interaction: CommandInteraction) => {
    const text = interaction.options.get(rosetty.t('talkCommandOptionText')!)?.value as string | null;
    const voice = interaction.options.get(rosetty.t('talkCommandOptionVoice')!)?.value as string;

    let filePath: string | null = null;

    try {
      filePath = await promisedGtts(voice, rosetty.getCurrentLang());
      const sourceHash = crypto
        .createHash('sha1')
        .update(`${rosetty.getCurrentLang()}:${voice}`)
        .digest('hex');

      const mediaAsset = await ingestMediaFromLocalFile(filePath, `gtts:${rosetty.getCurrentLang()}:${sourceHash}`);

      await createPlaybackJob({
        guildId: interaction.guildId!,
        mediaAsset,
        text,
        showText: !!text,
        authorName: interaction.user.username,
        authorImage: interaction.user.avatarURL(),
      });

      await interaction.reply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('success')!)
            .setDescription(rosetty.t('talkCommandAnswer')!)
            .setColor(0x2ecc71),
        ],
      });
    } catch (error) {
      if (!filePath) {
        logger.error(error, '[MEDIA] talk command failed (voice generation)');
      } else {
        const mediaError = toMediaIngestionError(error, 'TRANSCODE_FAILED');
        logger.error(mediaError, `[MEDIA] talk command failed (${mediaError.code})`);
      }

      const message = !filePath
        ? rosetty.t('talkCommandVoiceError')!
        : getLocalizedMediaErrorMessage(toMediaIngestionError(error, 'TRANSCODE_FAILED'));

      await interaction.reply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('error')!)
            .setDescription(message)
            .setColor(0xe74c3c),
        ],
        ephemeral: true,
      });
    } finally {
      if (filePath) {
        await deleteGtts(filePath).catch(() => undefined);
      }
    }
  },
});
