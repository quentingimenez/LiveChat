import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';
import { executeManualStopForGuild } from '../../services/manualStop';

export const stopCommand = (fastify: FastifyCustomInstance) => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('stopCommand')!)
    .setDescription(rosetty.t('stopCommandDescription')!),
  handler: async (interaction: CommandInteraction) => {
    await executeManualStopForGuild(fastify, interaction.guildId!, {
      logLabel: 'Stop command',
    });

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
