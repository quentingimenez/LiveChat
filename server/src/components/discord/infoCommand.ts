import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';

export const infoCommand = () => ({
  data: new SlashCommandBuilder().setName('info').setDescription('Bot Informations'),
  handler: async (interaction: CommandInteraction) => {
    await interaction.reply({
      embeds: [
        new EmbedBuilder()
          .setTitle(`LiveChat Server - ${new Date().getFullYear()}`)
          .setDescription('[GitHub Repository](https://github.com/quentingimenez/LiveChat-server)'),
      ],
      ephemeral: true,
    });
  },
});
