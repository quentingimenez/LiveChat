import { createReadStream } from 'fs';
import { unlink, writeFile } from 'fs/promises';
import { join } from 'path';
import fetch from 'node-fetch';

const GOOGLE_TTS_ENDPOINT = 'https://translate.google.com/translate_tts';
const MAX_TTS_CHUNK_LENGTH = 180;

const splitTextIntoChunks = (text: string): string[] => {
  const normalized = text.trim().replace(/\s+/g, ' ');

  if (!normalized) {
    return [];
  }

  const chunks: string[] = [];
  let cursor = normalized;

  while (cursor.length > MAX_TTS_CHUNK_LENGTH) {
    let cutIndex = cursor.lastIndexOf(' ', MAX_TTS_CHUNK_LENGTH);

    if (cutIndex < 1) {
      cutIndex = MAX_TTS_CHUNK_LENGTH;
    }

    chunks.push(cursor.slice(0, cutIndex).trim());
    cursor = cursor.slice(cutIndex).trim();
  }

  if (cursor.length > 0) {
    chunks.push(cursor);
  }

  return chunks;
};

const resolveLanguage = (lang: string): string => {
  const normalized = `${lang || ''}`.trim().toLowerCase();

  if (!normalized) {
    return 'en';
  }

  return normalized.split(/[-_]/)[0] || 'en';
};

const buildTtsUrl = (text: string, lang: string) => {
  const url = new URL(GOOGLE_TTS_ENDPOINT);
  url.searchParams.set('ie', 'UTF-8');
  url.searchParams.set('client', 'tw-ob');
  url.searchParams.set('tl', resolveLanguage(lang));
  url.searchParams.set('q', text);
  return url.toString();
};

const fetchTtsChunk = async (text: string, lang: string): Promise<Buffer> => {
  const response = await fetch(buildTtsUrl(text, lang), {
    headers: {
      'user-agent': 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36',
    },
  });

  if (!response.ok) {
    throw new Error(`TTS request failed with status ${response.status}`);
  }

  const audioBuffer = Buffer.from(await response.arrayBuffer());

  if (audioBuffer.length === 0) {
    throw new Error('TTS response is empty');
  }

  return audioBuffer;
};

export const promisedGtts = async (voice: string, lang: string): Promise<string> => {
  const chunks = splitTextIntoChunks(voice || '');

  if (chunks.length === 0) {
    throw new Error('TTS text is empty');
  }

  const buffers: Buffer[] = [];

  for (const chunk of chunks) {
    const chunkBuffer = await fetchTtsChunk(chunk, lang);
    buffers.push(chunkBuffer);
  }

  const filePath = join(__dirname, `${Date.now()}-${Math.ceil(Math.random() * 100)}.mp3`);
  await writeFile(filePath, Buffer.concat(buffers));
  return filePath;
};

export const readGttsAsStream = (filePath: string) => {
  return createReadStream(filePath);
};

export const deleteGtts = async (filePath: string) => {
  await unlink(filePath);
};
