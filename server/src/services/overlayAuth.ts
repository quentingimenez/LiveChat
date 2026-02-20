import crypto from 'crypto';
import type { FastifyRequest } from 'fastify';

interface CreateOverlayClientTokenParams {
  guildId: string;
  label: string;
  defaultAuthorName?: string | null;
  defaultAuthorImage?: string | null;
  createdByDiscordUserId?: string | null;
}

export interface OverlayClientRecord {
  id: string;
  guildId: string;
  label: string;
  tokenHash: string;
  defaultAuthorName: string | null;
  defaultAuthorImage: string | null;
  createdByDiscordUserId: string | null;
  lastSeenAt: Date | null;
  createdAt: Date;
  revokedAt: Date | null;
}

export const hashOverlayToken = (token: string) => {
  return crypto.createHash('sha256').update(token).digest('hex');
};

export const generateOverlayToken = () => {
  return crypto.randomBytes(32).toString('hex');
};

export const createOverlayClientToken = async (params: CreateOverlayClientTokenParams) => {
  const rawToken = generateOverlayToken();
  const tokenHash = hashOverlayToken(rawToken);
  const createData: Record<string, unknown> = {
    guildId: params.guildId,
    label: params.label,
    tokenHash,
  };

  if (params.defaultAuthorName && params.defaultAuthorName.trim() !== '') {
    createData.defaultAuthorName = params.defaultAuthorName.trim();
  }

  if (params.defaultAuthorImage && params.defaultAuthorImage.trim() !== '') {
    createData.defaultAuthorImage = params.defaultAuthorImage.trim();
  }

  if (params.createdByDiscordUserId && params.createdByDiscordUserId.trim() !== '') {
    createData.createdByDiscordUserId = params.createdByDiscordUserId.trim();
  }

  let client: OverlayClientRecord;

  try {
    client = (await prisma.overlayClient.create({
      data: createData as any,
    })) as OverlayClientRecord;
  } catch (error) {
    const errorMessage = `${(error as Error)?.message || ''}`;
    const hasAuthorMetadata =
      !!createData.defaultAuthorName || !!createData.defaultAuthorImage || !!createData.createdByDiscordUserId;
    const shouldRetryLegacyCreate = hasAuthorMetadata && errorMessage.includes('Unknown argument');

    if (!shouldRetryLegacyCreate) {
      throw error;
    }

    client = (await prisma.overlayClient.create({
      data: {
        guildId: params.guildId,
        label: params.label,
        tokenHash,
      } as any,
    })) as OverlayClientRecord;
  }

  return {
    client,
    rawToken,
  };
};

export const getBearerTokenFromRequest = (request: FastifyRequest): string | null => {
  const authHeader = request.headers.authorization;

  if (!authHeader) {
    return null;
  }

  const [scheme, token] = authHeader.split(' ');

  if (scheme?.toLowerCase() !== 'bearer' || !token) {
    return null;
  }

  return token;
};

export const resolveOverlayClientFromRequest = async (request: FastifyRequest) => {
  const bearerToken = getBearerTokenFromRequest(request);
  const queryToken = (request.query as Record<string, unknown> | undefined)?.token;

  const token = bearerToken || (typeof queryToken === 'string' ? queryToken : null);

  if (!token) {
    return null;
  }

  const tokenHash = hashOverlayToken(token);

  const client = (await prisma.overlayClient.findFirst({
    where: {
      tokenHash,
      revokedAt: null,
    },
  })) as OverlayClientRecord | null;

  if (!client) {
    return null;
  }

  return {
    client,
    token,
  };
};
