import { execFile } from 'child_process';
import { createWriteStream, promises as fsPromises } from 'fs';
import { once } from 'events';
import path from 'path';
import os from 'os';
import { promisify } from 'util';
import fetch from 'node-fetch';
import mime from 'mime-types';
import { addHours } from 'date-fns';
import { MediaAssetKind, MediaAssetStatus } from '../prisma/prismaEnums';
import {
  buildMediaOutputBasePath,
  ensureMediaStorageDir,
  getReadyCachedMediaAsset,
  touchMediaAsset,
} from './mediaCache';
import {
  MediaIngestionError,
  getMediaErrorCodeFromHttpStatus,
  pickMostRelevantMediaError,
  toMediaIngestionError,
} from './mediaErrors';
import { normalizeDownloadedMedia } from './mediaTranscode';
import { buildSourceHash, canonicalizeSourceUrl, resolveMediaSource } from './mediaSourceResolver';

const execFileAsync = promisify(execFile);
const BYTES_PER_MEGABYTE = 1024 * 1024;
const LEGACY_YTDLP_FORMAT = 'bv*[height<=1080]+ba/b[height<=1080]/best';
const PREVIOUS_COMPAT_YTDLP_FORMAT =
  'bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best';
const PREVIOUS_PROGRESSIVE_FIRST_COMPAT_YTDLP_FORMAT =
  'b[ext=mp4][height<=1080]/bv*[vcodec^=avc1][ext=mp4][height<=1080]+ba[ext=m4a]/b[vcodec^=avc1][ext=mp4][height<=1080]/bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best';
const COMPAT_YTDLP_FORMAT =
  'bv*[vcodec^=avc1][ext=mp4][height<=1080]+ba[ext=m4a]/b[vcodec^=avc1][ext=mp4][height<=1080]/bv*[ext=mp4][height<=1080]+ba[ext=m4a]/b[ext=mp4][height<=1080]/bv*[height<=1080]+ba/b[height<=1080]/best';

const getMaxMediaSizeBytes = () => Math.max(1, env.MEDIA_MAX_SIZE_MB) * BYTES_PER_MEGABYTE;

const resolveYtdlpFormatSelector = () => {
  const formatSelector = (env.YTDLP_FORMAT || '').trim();

  if (
    formatSelector === LEGACY_YTDLP_FORMAT ||
    formatSelector === PREVIOUS_COMPAT_YTDLP_FORMAT ||
    formatSelector === PREVIOUS_PROGRESSIVE_FIRST_COMPAT_YTDLP_FORMAT
  ) {
    return COMPAT_YTDLP_FORMAT;
  }

  return formatSelector;
};

const newFileTooLargeError = (sourceUrl: string, details?: string) => {
  return new MediaIngestionError(
    'FILE_TOO_LARGE',
    `Media exceeds max size of ${Math.max(1, env.MEDIA_MAX_SIZE_MB)} MB`,
    details || `Media exceeds configured max size for ${sourceUrl}`,
  );
};

const ensureFileSizeWithinLimit = async (filePath: string, sourceUrl: string) => {
  const fileStats = await fsPromises.stat(filePath);

  if (fileStats.size > getMaxMediaSizeBytes()) {
    throw newFileTooLargeError(sourceUrl, `Downloaded file is too large (${fileStats.size} bytes) for ${sourceUrl}`);
  }
};

const defaultAssetCreateData = {
  kind: MediaAssetKind.VIDEO,
  mime: 'application/octet-stream',
  storagePath: '',
  sizeBytes: 0,
};

const createTempDir = async () => {
  return fsPromises.mkdtemp(path.join(os.tmpdir(), 'livechat-media-'));
};

const parseYtdlpFilename = (stdout: string): string | null => {
  const lines = stdout
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);

  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const line = lines[index];

    if (!line.startsWith('{') || !line.endsWith('}')) {
      continue;
    }

    try {
      const payload = JSON.parse(line);
      if (typeof payload?._filename === 'string' && payload._filename.trim() !== '') {
        return payload._filename;
      }
    } catch {
      // Keep scanning previous lines.
    }
  }

  return null;
};

const findDownloadedFile = async (tmpDir: string) => {
  const entries = await fsPromises.readdir(tmpDir, {
    withFileTypes: true,
  });

  const files = entries
    .filter((entry) => entry.isFile())
    .map((entry) => path.join(tmpDir, entry.name))
    .filter((filePath) => !filePath.endsWith('.part'));

  if (files.length === 0) {
    return null;
  }

  const sortedByMtime = await Promise.all(
    files.map(async (filePath) => ({
      filePath,
      stat: await fsPromises.stat(filePath),
    })),
  );

  sortedByMtime.sort((a, b) => b.stat.mtimeMs - a.stat.mtimeMs);

  return sortedByMtime[0].filePath;
};

const downloadWithYtDlp = async (sourceUrl: string, tmpDir: string): Promise<string> => {
  const outputTemplate = path.join(tmpDir, 'download.%(ext)s');
  const formatSelector = resolveYtdlpFormatSelector();
  const concurrentFragments = Math.max(1, env.YTDLP_CONCURRENT_FRAGMENTS);
  const args = [
    '--no-playlist',
    '--no-progress',
    '--no-warnings',
    '--print-json',
    '--merge-output-format',
    'mp4',
    '--max-filesize',
    `${Math.max(1, env.MEDIA_MAX_SIZE_MB)}M`,
  ];

  if (concurrentFragments > 1) {
    args.push('--concurrent-fragments', `${concurrentFragments}`);
  }

  if (formatSelector.length > 0) {
    args.push('--format', formatSelector);
  }

  args.push('-o', outputTemplate, sourceUrl);

  let stdout = '';
  try {
    const result = await execFileAsync(env.YTDLP_BINARY, args, {
      timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
      maxBuffer: 10 * 1024 * 1024,
    });
    stdout = result.stdout || '';
  } catch (error) {
    throw toMediaIngestionError(error);
  }

  const filenameFromStdout = parseYtdlpFilename(stdout);

  if (filenameFromStdout) {
    await ensureFileSizeWithinLimit(filenameFromStdout, sourceUrl);
    return filenameFromStdout;
  }

  const downloadedFile = await findDownloadedFile(tmpDir);

  if (!downloadedFile) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'yt-dlp completed but no file was produced',
      `yt-dlp completed but no file was produced for ${sourceUrl}`,
    );
  }

  await ensureFileSizeWithinLimit(downloadedFile, sourceUrl);
  return downloadedFile;
};

const downloadWithHttp = async (sourceUrl: string, tmpDir: string): Promise<string> => {
  const response = await fetch(sourceUrl).catch((error) => {
    throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
  });

  if (!response.ok) {
    throw new MediaIngestionError(
      getMediaErrorCodeFromHttpStatus(response.status),
      `Unable to download media (${response.status})`,
      `HTTP download failed with status ${response.status} for ${sourceUrl}`,
    );
  }

  const maxSizeBytes = getMaxMediaSizeBytes();
  const contentLength = Number.parseInt(response.headers.get('content-length') || '', 10);

  if (Number.isFinite(contentLength) && contentLength > maxSizeBytes) {
    throw newFileTooLargeError(sourceUrl, `HTTP content-length is too large (${contentLength} bytes) for ${sourceUrl}`);
  }

  const contentType = response.headers.get('content-type') || 'application/octet-stream';
  const extension = mime.extension(contentType) || 'bin';
  const outputPath = path.join(tmpDir, `download-direct.${extension}`);
  const stream = response.body;

  if (!stream) {
    throw new MediaIngestionError('DOWNLOAD_FAILED', 'Unable to read media stream', `Empty HTTP body for ${sourceUrl}`);
  }

  let downloadedBytes = 0;
  const outputFile = createWriteStream(outputPath);

  try {
    for await (const chunk of stream as AsyncIterable<Buffer>) {
      const bufferChunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      downloadedBytes += bufferChunk.length;

      if (downloadedBytes > maxSizeBytes) {
        throw newFileTooLargeError(
          sourceUrl,
          `HTTP streamed media is too large (${downloadedBytes} bytes) for ${sourceUrl}`,
        );
      }

      if (!outputFile.write(bufferChunk)) {
        await once(outputFile, 'drain');
      }
    }

    outputFile.end();
    await once(outputFile, 'close');
  } catch (error) {
    const abortableStream = stream as unknown as { destroy?: () => void };
    if (typeof abortableStream.destroy === 'function') {
      abortableStream.destroy();
    }
    outputFile.destroy();
    await fsPromises.rm(outputPath, { force: true }).catch(() => undefined);

    if (error instanceof MediaIngestionError) {
      throw error;
    }

    throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
  }

  await ensureFileSizeWithinLimit(outputPath, sourceUrl);

  return outputPath;
};

const downloadSourceToTempFile = async (sourceUrl: string, tmpDir: string): Promise<string> => {
  let ytdlpError: unknown = null;

  try {
    return await downloadWithYtDlp(sourceUrl, tmpDir);
  } catch (error) {
    ytdlpError = error;
    const normalized = toMediaIngestionError(error);
    if (normalized.code === 'FILE_TOO_LARGE') {
      throw normalized;
    }
    logger.warn(`[MEDIA] yt-dlp fallback for ${sourceUrl} (${normalized.code})`);
  }

  try {
    return await downloadWithHttp(sourceUrl, tmpDir);
  } catch (httpError) {
    throw pickMostRelevantMediaError(ytdlpError, httpError);
  }
};

const removeExistingOutputVariants = async (outputBasePath: string) => {
  const outputDir = path.dirname(outputBasePath);
  const outputPrefix = `${path.basename(outputBasePath)}.`;

  const files = await fsPromises.readdir(outputDir, { withFileTypes: true });

  await Promise.all(
    files
      .filter((file) => file.isFile() && file.name.startsWith(outputPrefix))
      .map((file) => fsPromises.rm(path.join(outputDir, file.name), { force: true })),
  );
};

const upsertProcessingAsset = async (sourceHash: string, sourceUrl: string) => {
  const expiresAt = addHours(new Date(), Math.max(1, env.MEDIA_CACHE_TTL_HOURS));

  return prisma.mediaAsset.upsert({
    where: {
      sourceHash,
    },
    create: {
      sourceHash,
      sourceUrl,
      ...defaultAssetCreateData,
      expiresAt,
      status: MediaAssetStatus.PROCESSING,
      lastAccessedAt: new Date(),
    },
    update: {
      sourceUrl,
      status: MediaAssetStatus.PROCESSING,
      error: null,
      expiresAt,
      lastAccessedAt: new Date(),
    },
  });
};

const markAssetFailed = async (sourceHash: string, error: MediaIngestionError) => {
  const persistedMessage = `[${error.code}] ${error.message}`.slice(0, 500);

  await prisma.mediaAsset.updateMany({
    where: {
      sourceHash,
    },
    data: {
      status: MediaAssetStatus.FAILED,
      error: persistedMessage,
      expiresAt: addHours(new Date(), 1),
    },
  });
};

const toMediaKind = (kind: 'image' | 'audio' | 'video'): MediaAssetKind => {
  if (kind === 'image') {
    return MediaAssetKind.IMAGE;
  }

  if (kind === 'audio') {
    return MediaAssetKind.AUDIO;
  }

  return MediaAssetKind.VIDEO;
};

const normalizeAndPersistAsset = async (params: { sourceHash: string; sourceUrl: string; inputFilePath: string }) => {
  await ensureMediaStorageDir();

  const outputBasePath = buildMediaOutputBasePath(params.sourceHash);

  await removeExistingOutputVariants(outputBasePath);

  const normalized = await normalizeDownloadedMedia(params.inputFilePath, outputBasePath);

  return prisma.mediaAsset.update({
    where: {
      sourceHash: params.sourceHash,
    },
    data: {
      sourceUrl: params.sourceUrl,
      kind: toMediaKind(normalized.kind),
      mime: normalized.mime,
      durationSec: normalized.durationSec,
      width: normalized.width,
      height: normalized.height,
      isVertical: normalized.isVertical,
      storagePath: normalized.storagePath,
      sizeBytes: normalized.sizeBytes,
      status: MediaAssetStatus.READY,
      error: null,
      lastAccessedAt: new Date(),
      expiresAt: addHours(new Date(), Math.max(1, env.MEDIA_CACHE_TTL_HOURS)),
    },
  });
};

const ingestFromSourceUrlInternal = async (sourceUrl: string, sourceHash: string) => {
  const cached = await getReadyCachedMediaAsset(sourceHash);

  if (cached) {
    return touchMediaAsset(cached.id);
  }

  await upsertProcessingAsset(sourceHash, sourceUrl);

  const tmpDir = await createTempDir();
  const startedAtMs = Date.now();

  try {
    const downloadedFilePath = await downloadSourceToTempFile(sourceUrl, tmpDir);
    const downloadedAtMs = Date.now();

    const normalizedAsset = await normalizeAndPersistAsset({
      sourceHash,
      sourceUrl,
      inputFilePath: downloadedFilePath,
    });
    const finishedAtMs = Date.now();

    logger.info(
      `[MEDIA] Ingested ${sourceHash.slice(0, 8)} in ${finishedAtMs - startedAtMs}ms (download ${
        downloadedAtMs - startedAtMs
      }ms, normalize ${finishedAtMs - downloadedAtMs}ms)`,
    );

    return normalizedAsset;
  } catch (error) {
    const mediaError = toMediaIngestionError(error);
    await markAssetFailed(sourceHash, mediaError);
    throw mediaError;
  } finally {
    await fsPromises.rm(tmpDir, { recursive: true, force: true });
  }
};

const ingestFromLocalFileInternal = async (filePath: string, virtualSource: string) => {
  const canonicalSource = canonicalizeSourceUrl(virtualSource);
  const sourceHash = buildSourceHash(canonicalSource);

  const cached = await getReadyCachedMediaAsset(sourceHash);

  if (cached) {
    return touchMediaAsset(cached.id);
  }

  await upsertProcessingAsset(sourceHash, canonicalSource);

  const tmpDir = await createTempDir();

  try {
    const fileExtension = path.extname(filePath);
    const copiedPath = path.join(tmpDir, `local-source${fileExtension || '.bin'}`);

    await fsPromises.copyFile(filePath, copiedPath);

    const normalizedAsset = await normalizeAndPersistAsset({
      sourceHash,
      sourceUrl: canonicalSource,
      inputFilePath: copiedPath,
    });

    return normalizedAsset;
  } catch (error) {
    const mediaError = toMediaIngestionError(error, 'TRANSCODE_FAILED');
    await markAssetFailed(sourceHash, mediaError);
    throw mediaError;
  } finally {
    await fsPromises.rm(tmpDir, { recursive: true, force: true });
  }
};

export const ingestMediaFromSource = async (params: { url?: string | null; media?: string | null }) => {
  const resolvedSource = resolveMediaSource(params);

  if (!resolvedSource) {
    return null;
  }

  return ingestFromSourceUrlInternal(resolvedSource.sourceUrl, resolvedSource.sourceHash);
};

export const ingestMediaFromLocalFile = async (filePath: string, virtualSource: string) => {
  return ingestFromLocalFileInternal(filePath, virtualSource);
};
