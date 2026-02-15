import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { PlaybackJobStatus } from '../../services/prisma/prismaEnums';
import { OVERLAY_SOCKET_EVENTS } from '@livechat/overlay-protocol';

export const stopCommand = (fastify: FastifyCustomInstance) => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('stopCommand')!)
    .setDescription(rosetty.t('stopCommandDescription')!),
  handler: async (interaction: CommandInteraction) => {
    fastify.io.to(`overlay-guild-${interaction.guildId!}`).emit(OVERLAY_SOCKET_EVENTS.STOP, { jobId: 'manual-stop' });

    await prisma.guild.upsert({
      where: {
        id: interaction.guildId!,
      },
      create: {
        id: interaction.guildId!,
        busyUntil: null,
      },
      update: {
        busyUntil: null,
      },
    });

    const releasedJobs = await prisma.playbackJob.updateMany({
      where: {
        guildId: interaction.guildId!,
        status: PlaybackJobStatus.PLAYING,
        finishedAt: null,
      },
      data: {
        status: PlaybackJobStatus.DONE,
        finishedAt: new Date(),
      },
    });

    logger.info(
      `[PLAYBACK] Stop command released ${releasedJobs.count} playing job(s) for guild ${interaction.guildId!}`,
    );

    await interaction.reply({
      embeds: [
        new EmbedBuilder()
          .setTitle(rosetty.t('success')!)
          .setDescription(rosetty.t('stopCommandAnswer')!)
          .setColor(0x2ecc71),
      ],
      ephemeral: true,
    });
  },
});
