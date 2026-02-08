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

interface ProbedMediaDetails extends ProbedMetadata {
  formatName: string | null;
  videoCodec: string | null;
  audioCodec: string | null;
  pixelFormat: string | null;
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

const probeMediaDetails = async (filePath: string): Promise<ProbedMediaDetails> => {
  const { stdout } = await execFileAsync(
    env.FFPROBE_BINARY,
    [
      '-v',
      'error',
      '-show_entries',
      'stream=codec_type,codec_name,width,height,pix_fmt:format=duration,format_name',
      '-of',
      'json',
      filePath,
    ],
    {
      timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
      maxBuffer: 10 * 1024 * 1024,
    },
  );

  const parsed = JSON.parse(stdout || '{}');
  const streams = Array.isArray(parsed.streams) ? parsed.streams : [];
  const firstVideo = streams.find(
    (stream) => stream?.codec_type === 'video' || (typeof stream?.width === 'number' && typeof stream?.height === 'number'),
  );
  const firstAudio = streams.find((stream) => stream?.codec_type === 'audio');

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
    formatName: typeof parsed?.format?.format_name === 'string' ? parsed.format.format_name : null,
    videoCodec: typeof firstVideo?.codec_name === 'string' ? firstVideo.codec_name : null,
    audioCodec: typeof firstAudio?.codec_name === 'string' ? firstAudio.codec_name : null,
    pixelFormat: typeof firstVideo?.pix_fmt === 'string' ? firstVideo.pix_fmt : null,
  };
};

const probeMedia = async (filePath: string): Promise<ProbedMetadata> => {
  const details = await probeMediaDetails(filePath);

  return {
    durationSec: details.durationSec,
    width: details.width,
    height: details.height,
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
  const maxHeight = Math.max(0, env.MEDIA_VIDEO_MAX_HEIGHT);
  const details = await probeMediaDetails(inputPath).catch(() => null);
  const isWithinTargetHeight = maxHeight === 0 || details?.height === null || (details?.height ?? 0) <= maxHeight;
  const isMp4Container =
    typeof details?.formatName === 'string' &&
    (details.formatName.includes('mp4') || details.formatName.includes('mov') || details.formatName.includes('m4a'));
  const canCopyWithoutTranscode =
    !!details &&
    isMp4Container &&
    isWithinTargetHeight &&
    details.videoCodec === 'h264' &&
    (details.audioCodec === null || details.audioCodec === 'aac') &&
    (details.pixelFormat === null || details.pixelFormat === 'yuv420p');

  if (canCopyWithoutTranscode) {
    await copyFile(inputPath, outputPath);
    return finalizeFile(outputPath, 'video', 'video/mp4');
  }

  const ffmpegArgs = [
    '-y',
    '-i',
    inputPath,
    '-c:v',
    'libx264',
    '-preset',
    (env.MEDIA_VIDEO_PRESET || 'superfast').trim() || 'superfast',
    '-pix_fmt',
    'yuv420p',
    '-movflags',
    '+faststart',
    '-c:a',
    'aac',
    '-b:a',
    '192k',
  ];

  if (maxHeight > 0) {
    ffmpegArgs.push('-vf', `scale=-2:${maxHeight}:force_original_aspect_ratio=decrease`);
  }

  ffmpegArgs.push(outputPath);

  await runFfmpeg(ffmpegArgs);

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
