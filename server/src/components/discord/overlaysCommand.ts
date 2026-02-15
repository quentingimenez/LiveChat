import { CommandInteraction, EmbedBuilder, SlashCommandBuilder } from 'discord.js';

export const overlaysCommand = (fastify: FastifyCustomInstance) => ({
  data: new SlashCommandBuilder()
    .setName(rosetty.t('overlaysCommand')!)
    .setDescription(rosetty.t('overlaysCommandDescription')!),
  handler: async (interaction: CommandInteraction) => {
    const guildId = interaction.guildId;

    if (!guildId) {
      await interaction.reply({
        embeds: [
          new EmbedBuilder().setTitle(rosetty.t('error')!).setDescription(rosetty.t('notAllowed')!).setColor(0xe74c3c),
        ],
        ephemeral: true,
      });
      return;
    }

    const roomName = `overlay-guild-${guildId}`;
    const sockets = await fastify.io.in(roomName).fetchSockets();

    if (sockets.length === 0) {
      await interaction.reply({
        embeds: [
          new EmbedBuilder()
            .setTitle(rosetty.t('overlaysCommandAnswerTitle', { count: 0 })!)
            .setDescription(rosetty.t('overlaysCommandAnswerEmpty')!)
            .setColor(0xf1c40f),
        ],
        ephemeral: true,
      });
      return;
    }

    const connectedOverlays = sockets
      .map((socket) => ({
        label:
          typeof socket.data?.overlayClientLabel === 'string' && socket.data.overlayClientLabel.trim()
            ? socket.data.overlayClientLabel.trim()
            : 'unknown-device',
        clientId:
          typeof socket.data?.overlayClientId === 'string' && socket.data.overlayClientId.trim()
            ? socket.data.overlayClientId.trim()
            : 'unknown-client',
      }))
      .sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }));

    const description = connectedOverlays
      .map((overlay) => `• \`${overlay.label}\` (\`${overlay.clientId}\`)`)
      .join('\n');

    await interaction.reply({
      embeds: [
        new EmbedBuilder()
          .setTitle(rosetty.t('overlaysCommandAnswerTitle', { count: connectedOverlays.length })!)
          .setDescription(description)
          .setColor(0x2ecc71),
      ],
      ephemeral: true,
    });
  },
});
