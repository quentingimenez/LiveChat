import 'dotenv/config';
import { createEnv } from '@t3-oss/env-core';
import { z } from 'zod';

export const env = createEnv({
  server: {
    NODE_ENV: z.string().default('development'),
    LOG: z.enum(['info', 'debug', 'error', 'silent', 'warning']).default('info'),
    PORT: z
      .string()
      .default('3000')
      .transform((s) => parseInt(s, 10)),

    I18N: z.string().default('fr'),

    API_URL: z.string().url(),

    DISCORD_TOKEN: z.string(),
    DISCORD_CLIENT_ID: z.string(),

    DATABASE_URL: z.string(),

    HIDE_COMMANDS_DISABLED: z.string().default('false'),
    DEFAULT_DURATION: z
      .string()
      .default('5')
      .transform((s) => parseInt(s, 10)),

    MEDIA_STORAGE_DIR: z.string().default('./data/media'),
    MEDIA_CACHE_TTL_HOURS: z
      .string()
      .default('12')
      .transform((s) => parseInt(s, 10)),
    MEDIA_DOWNLOAD_TIMEOUT_MS: z
      .string()
      .default('180000')
      .transform((s) => parseInt(s, 10)),
    MEDIA_MAX_SIZE_MB: z
      .string()
      .default('2048')
      .transform((s) => parseInt(s, 10)),
    MEDIA_VIDEO_MAX_HEIGHT: z
      .string()
      .default('1080')
      .transform((s) => parseInt(s, 10)),
    MEDIA_VIDEO_PRESET: z.string().default('superfast'),
    TWITTER_OEMBED_ENDPOINT: z.string().url().default('https://publish.twitter.com/oembed'),
    TWITTER_CARD_DURATION_SEC: z
      .string()
      .default('15')
      .transform((s) => parseInt(s, 10)),
    PAIRING_CODE_TTL_MINUTES: z
      .string()
      .default('5')
      .transform((s) => parseInt(s, 10)),
    YTDLP_BINARY: z.string().default('yt-dlp'),
    YTDLP_FORMAT: z.string().default('bv*[height<=1080]+ba/b[height<=1080]/best'),
    FFMPEG_BINARY: z.string().default('ffmpeg'),
    FFPROBE_BINARY: z.string().default('ffprobe'),
  },
  runtimeEnv: process.env,
});

export enum Environment {
  TEST = 'test',
  DEVELOPMENT = 'development',
  STAGING = 'staging',
  PREPRODUCTION = 'preproduction',
  PRODUCTION = 'production',
}

export const currentEnv = () =>
  (!!env.NODE_ENV && env.NODE_ENV !== undefined ? env.NODE_ENV : Environment.DEVELOPMENT)
    ?.toString()
    ?.toLowerCase()
    ?.trim();
export const isProductionEnv = () => currentEnv() === Environment.PRODUCTION;
export const isPreProductionEnv = () => currentEnv() === Environment.PREPRODUCTION;
export const isStagingEnv = () => currentEnv() === Environment.STAGING;
export const isDevelopmentEnv = () => currentEnv() === Environment.DEVELOPMENT;
export const isTestEnv = () => currentEnv() === Environment.TEST;
export const isDeployedEnv = () =>
  Object.values(Environment)
    .filter((v) => v !== Environment.TEST && v !== Environment.DEVELOPMENT)
    .indexOf(currentEnv() as Environment) !== -1;
