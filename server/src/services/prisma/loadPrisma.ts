import { PrismaClient } from '@prisma/client';

export const loadPrismaClient = async () => {
  const prisma = new PrismaClient();

  global.prisma = prisma;

  await prisma.$connect();
};
