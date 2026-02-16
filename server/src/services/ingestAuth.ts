import { timingSafeEqual } from 'crypto';
import type { FastifyRequest } from 'fastify';
import { env } from './env';
import { generateOverlayToken, getBearerTokenFromRequest, hashOverlayToken } from './overlayAuth';

export interface CreateIngestClientTokenParams {
  guildId: string;
  label: string;
  defaultAuthorName: string;
  defaultAuthorImage: string | null;
  createdByDiscordUserId: string;
}

export interface IngestClientRecord {
  id: string;
  guildId: string;
  label: string;
  tokenHash: string;
  defaultAuthorName: string;
  defaultAuthorImage: string | null;
  createdByDiscordUserId: string;
  lastSeenAt: Date | null;
  createdAt: Date;
  revokedAt: Date | null;
}

const getIngestClientDelegate = (): {
  create: (args: unknown) => Promise<unknown>;
  findFirst: (args: unknown) => Promise<unknown>;
  updateMany: (args: unknown) => Promise<unknown>;
} | null => {
  const delegate = (prisma as unknown as { ingestClient?: unknown }).ingestClient;

  if (!delegate || typeof delegate !== 'object') {
    return null;
  }

  return delegate as {
    create: (args: unknown) => Promise<unknown>;
    findFirst: (args: unknown) => Promise<unknown>;
    updateMany: (args: unknown) => Promise<unknown>;
  };
};

const isLegacyIngestToken = (rawToken: string): boolean => {
  const expectedToken = env.INGEST_API_TOKEN.trim();
  const providedToken = rawToken.trim();

  if (!expectedToken || !providedToken) {
    return false;
  }

  const expectedBuffer = Buffer.from(expectedToken);
  const providedBuffer = Buffer.from(providedToken);

  if (expectedBuffer.length !== providedBuffer.length) {
    return false;
  }

  return timingSafeEqual(expectedBuffer, providedBuffer);
};

export const createIngestClientToken = async (params: CreateIngestClientTokenParams) => {
  const delegate = getIngestClientDelegate();

  if (!delegate) {
    throw new Error('ingest_client_model_unavailable');
  }

  const rawToken = generateOverlayToken();
  const tokenHash = hashOverlayToken(rawToken);

  const client = (await delegate.create({
    data: {
      guildId: params.guildId,
      label: params.label,
      tokenHash,
      defaultAuthorName: params.defaultAuthorName,
      defaultAuthorImage: params.defaultAuthorImage,
      createdByDiscordUserId: params.createdByDiscordUserId,
    },
  })) as IngestClientRecord;

  return {
    client,
    rawToken,
  };
};

export const revokeIngestClientsForGuildLabel = async (guildId: string, label: string): Promise<void> => {
  const delegate = getIngestClientDelegate();

  if (!delegate) {
    return;
  }

  await delegate.updateMany({
    where: {
      guildId,
      label,
      revokedAt: null,
    },
    data: {
      revokedAt: new Date(),
    },
  });
};

export const resolveIngestClientFromRequest = async (
  request: FastifyRequest,
): Promise<{ client: IngestClientRecord } | null> => {
  const token = getBearerTokenFromRequest(request);
  const delegate = getIngestClientDelegate();

  if (!token || !delegate) {
    return null;
  }

  const tokenHash = hashOverlayToken(token);

  const client = (await delegate.findFirst({
    where: {
      tokenHash,
      revokedAt: null,
    },
  })) as IngestClientRecord | null;

  if (!client) {
    return null;
  }

  await delegate.updateMany({
    where: {
      id: client.id,
      revokedAt: null,
    },
    data: {
      lastSeenAt: new Date(),
    },
  });

  return {
    client,
  };
};

export type IngestAuthResult =
  | {
      kind: 'legacy';
    }
  | {
      kind: 'client';
      client: IngestClientRecord;
    };

export const resolveIngestAuthFromRequest = async (request: FastifyRequest): Promise<IngestAuthResult | null> => {
  const token = getBearerTokenFromRequest(request);

  if (!token) {
    return null;
  }

  if (isLegacyIngestToken(token)) {
    return {
      kind: 'legacy',
    };
  }

  const clientResult = await resolveIngestClientFromRequest(request);

  if (!clientResult) {
    return null;
  }

  return {
    kind: 'client',
    client: clientResult.client,
  };
};

export const isIngestApiEnabled = async (): Promise<boolean> => {
  if (env.INGEST_API_TOKEN.trim()) {
    return true;
  }

  const delegate = getIngestClientDelegate();

  if (!delegate) {
    return false;
  }

  const activeClient = (await delegate.findFirst({
    where: {
      revokedAt: null,
    },
    select: {
      id: true,
    },
  })) as { id?: string } | null;

  return !!activeClient;
};
