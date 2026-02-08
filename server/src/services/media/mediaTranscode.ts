import { execFile } from 'child_process';
import { promisify } from 'util';
import { copyFile, stat } from 'fs/promises';
import path from 'path';
import mime from 'mime-types';
import { fileTypeFromFile } from 'file-type';
import type { OverlayMediaKind } from '@livechat/overlay-protocol';

const execFileAsync = promisify(execFile);

interface ProbedMetadata {
  durationSec: number | null;
  width: number | null;
  height: number | null;
}

export interface NormalizedMedia {
  kind: OverlayMediaKind;
  mime: string;
  durationSec: number | null;
  width: number | null;
  height: number | null;
  isVertical: boolean;
  storagePath: string;
  sizeBytes: number;
}

const runFfmpeg = async (args: string[]) => {
  await execFileAsync(env.FFMPEG_BINARY, args, {
    timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
    maxBuffer: 10 * 1024 * 1024,
  });
};

const probeMedia = async (filePath: string): Promise<ProbedMetadata> => {
  const { stdout } = await execFileAsync(
    env.FFPROBE_BINARY,
    ['-v', 'error', '-show_entries', 'stream=width,height:format=duration', '-of', 'json', filePath],
    {
      timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
      maxBuffer: 10 * 1024 * 1024,
    },
  );

  const parsed = JSON.parse(stdout || '{}');
  const streams = Array.isArray(parsed.streams) ? parsed.streams : [];
  const firstVideo = streams.find((stream) => typeof stream?.width === 'number' && typeof stream?.height === 'number');

  const width = firstVideo?.width ?? null;
  const height = firstVideo?.height ?? null;

  let durationSec: number | null = null;

  if (parsed?.format?.duration !== undefined && parsed?.format?.duration !== null) {
    const parsedDuration = Math.ceil(Number(parsed.format.duration));
    durationSec = Number.isFinite(parsedDuration) ? parsedDuration : null;
  }

  return {
    durationSec,
    width,
    height,
  };
};

const detectMime = async (filePath: string) => {
  const fileType = await fileTypeFromFile(filePath);

  if (fileType?.mime) {
    return fileType.mime;
  }

  const extension = path.extname(filePath).replace('.', '').toLowerCase();

  const guessed = mime.lookup(extension);

  return typeof guessed === 'string' ? guessed : 'application/octet-stream';
};

const deriveKindFromMime = (mimeType: string): OverlayMediaKind => {
  if (mimeType.startsWith('image/')) {
    return 'image';
  }

  if (mimeType.startsWith('audio/')) {
    return 'audio';
  }

  return 'video';
};

const finalizeFile = async (storagePath: string, kind: OverlayMediaKind, mimeType: string): Promise<NormalizedMedia> => {
  const fileStats = await stat(storagePath);
  const metadata = await probeMedia(storagePath).catch(() => ({ durationSec: null, width: null, height: null }));

  return {
    kind,
    mime: mimeType,
    durationSec: metadata.durationSec,
    width: metadata.width,
    height: metadata.height,
    isVertical:
      metadata.height !== null && metadata.width !== null && metadata.width > 0 ? metadata.height > metadata.width : false,
    storagePath,
    sizeBytes: fileStats.size,
  };
};

const normalizeImage = async (inputPath: string, outputBasePath: string, mimeType: string): Promise<NormalizedMedia> => {
  const ext = mime.extension(mimeType) || path.extname(inputPath).replace('.', '') || 'png';
  const outputPath = `${outputBasePath}.${ext}`;

  await copyFile(inputPath, outputPath);

  return finalizeFile(outputPath, 'image', mimeType);
};

const normalizeAudio = async (inputPath: string, outputBasePath: string): Promise<NormalizedMedia> => {
  const outputPath = `${outputBasePath}.mp3`;

  await runFfmpeg(['-y', '-i', inputPath, '-vn', '-acodec', 'libmp3lame', '-b:a', '192k', outputPath]);

  return finalizeFile(outputPath, 'audio', 'audio/mpeg');
};

const normalizeVideo = async (inputPath: string, outputBasePath: string): Promise<NormalizedMedia> => {
  const outputPath = `${outputBasePath}.mp4`;

  await runFfmpeg([
    '-y',
    '-i',
    inputPath,
    '-c:v',
    'libx264',
    '-preset',
    'veryfast',
    '-pix_fmt',
    'yuv420p',
    '-movflags',
    '+faststart',
    '-c:a',
    'aac',
    '-b:a',
    '192k',
    outputPath,
  ]);

  return finalizeFile(outputPath, 'video', 'video/mp4');
};

export const normalizeDownloadedMedia = async (inputPath: string, outputBasePath: string): Promise<NormalizedMedia> => {
  const mimeType = await detectMime(inputPath);
  const kind = deriveKindFromMime(mimeType);

  if (kind === 'image') {
    return normalizeImage(inputPath, outputBasePath, mimeType);
  }

  if (kind === 'audio') {
    return normalizeAudio(inputPath, outputBasePath);
  }

  return normalizeVideo(inputPath, outputBasePath);
};
