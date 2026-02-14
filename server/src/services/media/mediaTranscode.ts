import { execFile } from 'child_process';
import { promisify } from 'util';
import { copyFile, rm, stat } from 'fs/promises';
import path from 'path';
import mime from 'mime-types';
import { fileTypeFromFile } from 'file-type';
import type { OverlayMediaKind } from '@livechat/overlay-protocol';

const execFileAsync = promisify(execFile);
const CPU_VIDEO_ENCODER = 'libx264';
const GPU_VIDEO_ENCODER = 'h264_nvenc';
let hasNvencEncoderPromise: Promise<boolean> | null = null;

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

const getExecFailureDetails = (error: unknown) => {
  const asText = (value: unknown): string => {
    return typeof value === 'string' ? value.trim() : '';
  };

  const errorRecord = (error || {}) as {
    message?: unknown;
    code?: unknown;
    signal?: unknown;
    stderr?: unknown;
    stdout?: unknown;
  };

  const stderrText = asText(errorRecord.stderr);
  const stdoutText = asText(errorRecord.stdout);

  return {
    message: asText(errorRecord.message),
    code: typeof errorRecord.code === 'string' || typeof errorRecord.code === 'number' ? errorRecord.code : undefined,
    signal: typeof errorRecord.signal === 'string' ? errorRecord.signal : undefined,
    stderrTail: stderrText ? stderrText.split('\n').slice(-20).join('\n').slice(-2000) : undefined,
    stdoutTail: stdoutText ? stdoutText.split('\n').slice(-20).join('\n').slice(-1000) : undefined,
  };
};

const hasNvencEncoder = async () => {
  if (hasNvencEncoderPromise) {
    return hasNvencEncoderPromise;
  }

  hasNvencEncoderPromise = execFileAsync(env.FFMPEG_BINARY, ['-hide_banner', '-encoders'], {
    timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
    maxBuffer: 10 * 1024 * 1024,
  })
    .then(({ stdout }) => {
      return /\bh264_nvenc\b/.test(stdout || '');
    })
    .catch((error) => {
      logger.warn(error, '[MEDIA] Unable to probe ffmpeg encoders, fallback to CPU');
      return false;
    });

  return hasNvencEncoderPromise;
};

const resolveVideoEncoder = async (): Promise<typeof CPU_VIDEO_ENCODER | typeof GPU_VIDEO_ENCODER> => {
  if (env.MEDIA_VIDEO_ENCODER === CPU_VIDEO_ENCODER) {
    return CPU_VIDEO_ENCODER;
  }

  const nvencAvailable = await hasNvencEncoder();

  if (!nvencAvailable) {
    if (env.MEDIA_VIDEO_ENCODER === GPU_VIDEO_ENCODER) {
      logger.warn('[MEDIA] MEDIA_VIDEO_ENCODER=h264_nvenc but encoder is unavailable, fallback to libx264');
    }
    return CPU_VIDEO_ENCODER;
  }

  if (env.MEDIA_VIDEO_ENCODER === 'auto') {
    return GPU_VIDEO_ENCODER;
  }

  return GPU_VIDEO_ENCODER;
};

const buildVideoTranscodeArgs = (params: {
  inputPath: string;
  outputPath: string;
  maxHeight: number;
  videoEncoder: typeof CPU_VIDEO_ENCODER | typeof GPU_VIDEO_ENCODER;
  cpuPreset: string;
  nvencPreset: string;
}) => {
  const ffmpegArgs = ['-y', '-i', params.inputPath, '-c:v', params.videoEncoder];

  if (params.videoEncoder === GPU_VIDEO_ENCODER) {
    ffmpegArgs.push('-preset', params.nvencPreset);
  } else {
    ffmpegArgs.push('-preset', params.cpuPreset);
  }

  ffmpegArgs.push('-pix_fmt', 'yuv420p', '-movflags', '+faststart', '-c:a', 'aac', '-b:a', '192k');

  if (params.maxHeight > 0) {
    ffmpegArgs.push('-vf', `scale=-2:${params.maxHeight}:force_original_aspect_ratio=decrease`);
  }

  ffmpegArgs.push(params.outputPath);

  return ffmpegArgs;
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
    (stream) =>
      stream?.codec_type === 'video' || (typeof stream?.width === 'number' && typeof stream?.height === 'number'),
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

const finalizeFile = async (
  storagePath: string,
  kind: OverlayMediaKind,
  mimeType: string,
): Promise<NormalizedMedia> => {
  const fileStats = await stat(storagePath);
  const metadata = await probeMedia(storagePath).catch(() => ({ durationSec: null, width: null, height: null }));

  return {
    kind,
    mime: mimeType,
    durationSec: metadata.durationSec,
    width: metadata.width,
    height: metadata.height,
    isVertical:
      metadata.height !== null && metadata.width !== null && metadata.width > 0
        ? metadata.height > metadata.width
        : false,
    storagePath,
    sizeBytes: fileStats.size,
  };
};

const normalizeImage = async (
  inputPath: string,
  outputBasePath: string,
  mimeType: string,
): Promise<NormalizedMedia> => {
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
  const configuredCpuPreset = (env.MEDIA_VIDEO_PRESET || '').trim();
  const selectedCpuPreset =
    configuredCpuPreset && configuredCpuPreset !== 'superfast' ? configuredCpuPreset : 'ultrafast';
  const selectedNvencPreset = (env.MEDIA_VIDEO_NVENC_PRESET || '').trim() || 'p4';
  const details = await probeMediaDetails(inputPath).catch(() => null);
  const isWithinTargetHeight = maxHeight === 0 || details?.height === null || (details?.height ?? 0) <= maxHeight;
  const isMp4Container =
    typeof details?.formatName === 'string' &&
    (details.formatName.includes('mp4') || details.formatName.includes('mov') || details.formatName.includes('m4a'));
  const hasCompatiblePixelFormat =
    details?.pixelFormat === null || details?.pixelFormat === 'yuv420p' || details?.pixelFormat === 'yuvj420p';
  const canCopyWithoutTranscode =
    !!details &&
    isMp4Container &&
    isWithinTargetHeight &&
    details.videoCodec === 'h264' &&
    (details.audioCodec === null || details.audioCodec === 'aac') &&
    hasCompatiblePixelFormat;

  if (canCopyWithoutTranscode) {
    await copyFile(inputPath, outputPath);
    return finalizeFile(outputPath, 'video', 'video/mp4');
  }

  if (details) {
    logger.info(
      `[MEDIA] Transcode required (codec=${details.videoCodec || 'unknown'}/${details.audioCodec || 'none'}, pix=${
        details.pixelFormat || 'unknown'
      }, format=${details.formatName || 'unknown'}, height=${details.height ?? 'unknown'}, maxHeight=${maxHeight})`,
    );
  } else {
    logger.info('[MEDIA] Transcode required (ffprobe details unavailable)');
  }

  let selectedEncoder = await resolveVideoEncoder();
  logger.info(`[MEDIA] Video encoder selected: ${selectedEncoder}`);

  let ffmpegArgs = buildVideoTranscodeArgs({
    inputPath,
    outputPath,
    maxHeight,
    videoEncoder: selectedEncoder,
    cpuPreset: selectedCpuPreset,
    nvencPreset: selectedNvencPreset,
  });

  try {
    await runFfmpeg(ffmpegArgs);
  } catch (error) {
    if (selectedEncoder !== GPU_VIDEO_ENCODER) {
      logger.error(
        {
          ...getExecFailureDetails(error),
          inputPath,
          outputPath,
          selectedEncoder,
          maxHeight,
          ffmpegArgs,
        },
        '[MEDIA] ffmpeg transcode failed',
      );
      throw error;
    }

    logger.warn(
      {
        ...getExecFailureDetails(error),
        inputPath,
        outputPath,
        selectedEncoder,
        maxHeight,
        ffmpegArgs,
      },
      '[MEDIA] NVENC transcode failed, retry with libx264',
    );
    selectedEncoder = CPU_VIDEO_ENCODER;
    ffmpegArgs = buildVideoTranscodeArgs({
      inputPath,
      outputPath,
      maxHeight,
      videoEncoder: selectedEncoder,
      cpuPreset: selectedCpuPreset,
      nvencPreset: selectedNvencPreset,
    });

    await rm(outputPath, { force: true }).catch(() => undefined);

    try {
      await runFfmpeg(ffmpegArgs);
    } catch (retryError) {
      logger.error(
        {
          ...getExecFailureDetails(retryError),
          inputPath,
          outputPath,
          selectedEncoder,
          maxHeight,
          ffmpegArgs,
        },
        '[MEDIA] ffmpeg transcode failed after CPU retry',
      );
      throw retryError;
    }
  }

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
