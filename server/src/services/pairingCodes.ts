const PAIRING_CODE_PURGE_INTERVAL_MS = 60 * 1000;

export const purgeStalePairingCodes = async (): Promise<number> => {
  const deleteResult = await prisma.pairingCode.deleteMany({
    where: {
      OR: [
        {
          usedAt: {
            not: null,
          },
        },
        {
          expiresAt: {
            lte: new Date(),
          },
        },
      ],
    },
  });

  return deleteResult.count;
};

export const startPairingCodePurgeWorker = () => {
  setInterval(async () => {
    try {
      const deletedCount = await purgeStalePairingCodes();
      if (deletedCount > 0) {
        logger.info(`[PAIRING] Purged ${deletedCount} expired/used pairing codes`);
      }
    } catch (error) {
      logger.error(error, '[PAIRING] Pairing code purge failed');
    }
  }, PAIRING_CODE_PURGE_INTERVAL_MS);
};
