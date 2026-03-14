import { syncAllActiveDiscordUsers } from '../services/discordUserSync';

const DISCORD_USER_SYNC_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

export const startDiscordUserSyncWorker = () => {
  // Initial sync after 1 minute (allows Discord bot to fully initialize)
  setTimeout(async () => {
    logger.info('[DISCORD_SYNC] Running initial user profile sync');
    await syncAllActiveDiscordUsers();
  }, 60 * 1000);

  setInterval(async () => {
    logger.info('[DISCORD_SYNC] Running scheduled daily user profile sync');
    await syncAllActiveDiscordUsers();
  }, DISCORD_USER_SYNC_INTERVAL_MS);
};
