import { promises as fsPromises } from 'fs';
import path from 'path';
import { env } from './env';

const normalizeTikTokCookie = (value: unknown): string => {
  if (typeof value !== 'string') {
    return '';
  }

  return value.replace(/^cookie:\s*/i, '').trim();
};

const ENV_FILE_PATH = path.resolve(process.cwd(), '.env');

const runtimeSettings = {
  tiktokCookie: normalizeTikTokCookie(env.TIKTOK_COOKIE),
};

const escapeEnvDoubleQuotedValue = (value: string): string => {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
};

const buildTikTokCookieEnvLine = (value: string): string => {
  if (!value) {
    return 'TIKTOK_COOKIE=';
  }

  return `TIKTOK_COOKIE="${escapeEnvDoubleQuotedValue(value)}"`;
};

const upsertEnvLine = (input: string, envLine: string): string => {
  const normalized = input.replace(/\r\n/g, '\n');
  const lineRegex = /^\s*TIKTOK_COOKIE\s*=.*$/m;

  if (lineRegex.test(normalized)) {
    return normalized.replace(lineRegex, envLine);
  }

  const trimmedEnd = normalized.replace(/\s*$/, '');
  if (!trimmedEnd) {
    return `${envLine}\n`;
  }

  return `${trimmedEnd}\n${envLine}\n`;
};

export const getRuntimeTikTokCookie = (): string => {
  return runtimeSettings.tiktokCookie;
};

export const setRuntimeTikTokCookie = (value: unknown): string => {
  const normalized = normalizeTikTokCookie(value);
  runtimeSettings.tiktokCookie = normalized;
  return normalized;
};

export const persistRuntimeTikTokCookieToEnvFile = async (value: unknown): Promise<string> => {
  const normalized = setRuntimeTikTokCookie(value);
  let currentContent = '';

  try {
    currentContent = await fsPromises.readFile(ENV_FILE_PATH, 'utf8');
  } catch (error) {
    const maybeError = error as NodeJS.ErrnoException;
    if (maybeError?.code !== 'ENOENT') {
      throw error;
    }
  }

  const envLine = buildTikTokCookieEnvLine(normalized);
  const nextContent = upsertEnvLine(currentContent, envLine);

  if (nextContent !== currentContent) {
    await fsPromises.writeFile(ENV_FILE_PATH, nextContent, 'utf8');
  }

  return normalized;
};
