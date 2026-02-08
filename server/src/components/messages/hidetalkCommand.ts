import crypto from 'crypto';
import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { deleteGtts, promisedGtts } from '../../services/gtts';
import { ingestMediaFromLocalFile } from '../../services/media/mediaIngestion';
import { getLocalizedMediaErrorMessage, toMediaIngestionError } from '../../services/media/mediaErrors';
import { createPlaybackJob } from '../../services/playbackJobs';

export const hideTalkCommand = () => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('hideTalkCommand')!)
    .setDescription(rosetty.t('hideTalkCommandDescription')!)
    .addStringOption((option) =>
      option
        .setName(rosetty.t('hideTalkCommandOptionVoice')!)
        .setDescription(rosetty.t('hideTalkCommandOptionVoiceDescription')!)
        .setRequired(true),
    )
    .addStringOption((option) =>
      option
        .setName(rosetty.t('hideTalkCommandOptionText')!)
        .setDescription(rosetty.t('hideTalkCommandOptionTextDescription')!),
    ),
  handler: async (interaction: CommandInteraction) => {
    const text = interaction.options.get(rosetty.t('hideTalkCommandOptionText')!)?.value as string | null;
    const voice = interaction.options.get(rosetty.t('hideTalkCommandOptionVoice')!)?.value as string;

    let filePath: string | null = null;
    await interaction.deferReply({
      ephemeral: true,
    });

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
      });

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('success')!)
            .setDescription(rosetty.t('hideTalkCommandAnswer')!)
            .setColor(0x2ecc71),
        ],
      });
    } catch (error) {
      if (!filePath) {
        logger.error(error, '[MEDIA] hide talk command failed (voice generation)');
      } else {
        const mediaError = toMediaIngestionError(error, 'TRANSCODE_FAILED');
        logger.error(mediaError, `[MEDIA] hide talk command failed (${mediaError.code})`);
      }

      const message = !filePath
        ? rosetty.t('talkCommandVoiceError')!
        : getLocalizedMediaErrorMessage(toMediaIngestionError(error, 'TRANSCODE_FAILED'));

      await interaction.editReply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('error')!)
            .setDescription(message)
            .setColor(0xe74c3c),
        ],
      });
    } finally {
      if (filePath) {
        await deleteGtts(filePath).catch(() => undefined);
      }
    }
  },
});
