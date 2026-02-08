import crypto from 'crypto';
import type { FastifyRequest } from 'fastify';

export const hashOverlayToken = (token: string) => {
  return crypto.createHash('sha256').update(token).digest('hex');
};

export const generateOverlayToken = () => {
  return crypto.randomBytes(32).toString('hex');
};

export const createOverlayClientToken = async (params: { guildId: string; label: string }) => {
  const rawToken = generateOverlayToken();
  const tokenHash = hashOverlayToken(rawToken);

  const client = await prisma.overlayClient.create({
    data: {
      guildId: params.guildId,
      label: params.label,
      tokenHash,
    },
  });

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

  const client = await prisma.overlayClient.findFirst({
    where: {
      tokenHash,
      revokedAt: null,
    },
  });

  if (!client) {
    return null;
  }

  return {
    client,
    token,
  };
};
