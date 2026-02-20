/* eslint-disable no-var */
import { Server } from 'http';
import {
  FastifyBaseLogger,
  FastifyInstance,
  RawReplyDefaultExpression,
  RawRequestDefaultExpression,
  RawServerDefault,
  FastifyLoggerInstance,
} from 'fastify';
import { PrismaClient } from '@prisma/client';

import { socketioServer } from 'fastify-socket.io';
import { Server as ServerSocketIo } from 'socket.io';
import { REST, Client } from 'discord.js';
import { env as ENV } from '../services/env';
import { RosettyI18n } from '../services/i18n/loader';
import type {
  OverlayErrorPayload,
  OverlayHeartbeatPayload,
  OverlayMemeTriggerPayload,
  OverlayPlaybackStatePayload,
  OverlayPlayPayload,
  OverlayStopPayload,
} from '@livechat/overlay-protocol';

declare global {
  namespace globalThis {
    var logger: FastifyLoggerInstance;
    var env: typeof ENV;
    var discordRest: REST;
    var discordClient: Client;
    var rosetty: RosettyI18n;
    var prisma: PrismaClient;
    var commandsLoaded: string[];
  }

  interface ClientToServerEvents {
    'overlay:heartbeat': (payload: OverlayHeartbeatPayload) => void;
    'overlay:error': (payload: OverlayErrorPayload) => void;
    'overlay:playback-state': (payload: OverlayPlaybackStatePayload) => void;
    'overlay:stop': (payload: OverlayStopPayload) => void;
    'overlay:meme-trigger': (payload: OverlayMemeTriggerPayload) => void;
  }

  interface ServerToClientEvents {
    'overlay:play': (payload: OverlayPlayPayload) => void;
    'overlay:stop': (payload: OverlayStopPayload) => void;
  }

  interface InterServerEvents {}

  interface SocketData {
    guildId?: string;
    overlayClientId?: string;
    overlayClientLabel?: string;
  }

  type FastifyICustom = FastifyInstance<
    RawServerDefault,
    RawRequestDefaultExpression<Server>,
    RawReplyDefaultExpression<Server>,
    FastifyBaseLogger
  >;

  interface FastifyCustomInstance extends FastifyICustom, fastifySensible, socketioServer {
    io: ServerSocketIo<ClientToServerEvents, ServerToClientEvents, InterServerEvents, SocketData>;
  }
}
