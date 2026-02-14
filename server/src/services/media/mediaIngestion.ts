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

const TIKTOK_PAGE_USER_AGENT =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36';

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  return value as Record<string, unknown>;
};

const asNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();

  if (!normalized) {
    return null;
  }

  return normalized;
};

const isTikTokUrl = (sourceUrl: string): boolean => {
  try {
    const parsed = new URL(sourceUrl);
    return parsed.hostname.includes('tiktok.com');
  } catch {
    return false;
  }
};

const downloadHttpResponseToTempFile = async (params: {
  response: Awaited<ReturnType<typeof fetch>>;
  tmpDir: string;
  sourceUrl: string;
  outputBasename: string;
}) => {
  const maxSizeBytes = getMaxMediaSizeBytes();
  const contentLength = Number.parseInt(params.response.headers.get('content-length') || '', 10);

  if (Number.isFinite(contentLength) && contentLength > maxSizeBytes) {
    throw newFileTooLargeError(
      params.sourceUrl,
      `HTTP content-length is too large (${contentLength} bytes) for ${params.sourceUrl}`,
    );
  }

  const contentType = params.response.headers.get('content-type') || 'application/octet-stream';
  const extension = mime.extension(contentType) || 'bin';
  const outputPath = path.join(params.tmpDir, `${params.outputBasename}.${extension}`);
  const stream = params.response.body;

  if (!stream) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unable to read media stream',
      `Empty HTTP body for ${params.sourceUrl}`,
    );
  }

  let downloadedBytes = 0;
  const outputFile = createWriteStream(outputPath);

  try {
    for await (const chunk of stream as AsyncIterable<Buffer>) {
      const bufferChunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      downloadedBytes += bufferChunk.length;

      if (downloadedBytes > maxSizeBytes) {
        throw newFileTooLargeError(
          params.sourceUrl,
          `HTTP streamed media is too large (${downloadedBytes} bytes) for ${params.sourceUrl}`,
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

  await ensureFileSizeWithinLimit(outputPath, params.sourceUrl);

  return outputPath;
};

const extractTikTokSetCookieHeaders = (response: Awaited<ReturnType<typeof fetch>>) => {
  const responseHeaders = response.headers as unknown as {
    get: (name: string) => string | null;
    raw?: () => Record<string, string[]>;
  };

  if (typeof responseHeaders.raw === 'function') {
    const rawHeaders = responseHeaders.raw();
    if (Array.isArray(rawHeaders['set-cookie']) && rawHeaders['set-cookie'].length > 0) {
      return rawHeaders['set-cookie'];
    }
  }

  const fallback = response.headers.get('set-cookie');

  if (!fallback) {
    return [];
  }

  return [fallback];
};

const buildCookieHeader = (setCookieHeaders: string[]) => {
  return setCookieHeaders
    .map((headerValue) => headerValue.split(';')[0]?.trim() || '')
    .filter(Boolean)
    .join('; ');
};

const parseTikTokUniversalData = (html: string) => {
  const scriptMatch = html.match(/<script[^>]+id="__UNIVERSAL_DATA_FOR_REHYDRATION__"[^>]*>([\s\S]*?)<\/script>/i);

  if (!scriptMatch || !scriptMatch[1]) {
    return null;
  }

  try {
    const parsed = JSON.parse(scriptMatch[1]);
    return asRecord(parsed);
  } catch {
    return null;
  }
};

const extractTikTokItemIdFromUrl = (rawUrl: string): string | null => {
  try {
    const parsed = new URL(rawUrl);
    const pathMatch = parsed.pathname.match(/\/(?:video|photo)\/(\d+)/i);
    const fromPath = asNonEmptyString(pathMatch?.[1]);

    if (fromPath) {
      return fromPath;
    }
  } catch {
    // Ignore invalid URLs.
  }

  const genericMatch = rawUrl.match(/\b(\d{15,22})\b/);
  return asNonEmptyString(genericMatch?.[1]);
};

const extractTikTokCanonicalUrlFromHtml = (html: string): string | null => {
  const universalData = parseTikTokUniversalData(html);
  if (!universalData) {
    return null;
  }

  const defaultScope = asRecord(universalData['__DEFAULT_SCOPE__']);
  const seoAbtest = asRecord(defaultScope?.['seo.abtest']);
  return asNonEmptyString(seoAbtest?.canonical);
};

const extractTikTokMediaUrlsFromHtml = (html: string): string[] => {
  const universalData = parseTikTokUniversalData(html);
  if (!universalData) {
    return [];
  }

  const defaultScope = asRecord(universalData['__DEFAULT_SCOPE__']);
  const videoDetail = asRecord(defaultScope?.['webapp.video-detail']);
  const itemInfo = asRecord(videoDetail?.itemInfo);
  const primaryItemStruct = asRecord(itemInfo?.itemStruct);
  const fallbackItemStruct = asRecord(asRecord(itemInfo?.itemInfo)?.itemStruct);
  const itemStruct = primaryItemStruct || fallbackItemStruct;
  const videoRecord = asRecord(itemStruct?.video);

  if (!videoRecord) {
    return [];
  }

  const candidateUrls = new Set<string>();
  const addCandidate = (candidate: unknown) => {
    const normalized = asNonEmptyString(candidate);
    if (!normalized) {
      return;
    }

    if (normalized.startsWith('http://') || normalized.startsWith('https://')) {
      candidateUrls.add(normalized);
    }
  };

  const addUrlsFromAddressRecord = (candidate: unknown) => {
    const addressRecord = asRecord(candidate);

    if (!addressRecord) {
      addCandidate(candidate);
      return;
    }

    addCandidate(addressRecord.Url);
    addCandidate(addressRecord.url);

    const preferredList = addressRecord.UrlList;
    if (Array.isArray(preferredList)) {
      preferredList.forEach((entry) => addCandidate(entry));
    }

    const secondaryList = addressRecord.urlList;
    if (Array.isArray(secondaryList)) {
      secondaryList.forEach((entry) => addCandidate(entry));
    }
  };

  addUrlsFromAddressRecord(videoRecord.downloadAddr);
  addUrlsFromAddressRecord(videoRecord.playAddr);
  addUrlsFromAddressRecord(videoRecord.DownloadAddrStruct);
  addUrlsFromAddressRecord(videoRecord.downloadAddrStruct);
  addUrlsFromAddressRecord(videoRecord.PlayAddrStruct);
  addUrlsFromAddressRecord(videoRecord.playAddrStruct);

  const bitrateInfo = videoRecord.bitrateInfo;

  if (Array.isArray(bitrateInfo)) {
    bitrateInfo.forEach((entry) => {
      const bitrateRecord = asRecord(entry);
      if (!bitrateRecord) {
        return;
      }

      addUrlsFromAddressRecord(bitrateRecord.PlayAddr);
      addUrlsFromAddressRecord(bitrateRecord.playAddr);
      addUrlsFromAddressRecord(bitrateRecord.DownloadAddr);
      addUrlsFromAddressRecord(bitrateRecord.downloadAddr);
    });
  }

  return [...candidateUrls];
};

const extractTikTokEmbedMediaUrlsFromHtml = (html: string): string[] => {
  const scriptMatch = html.match(/<script[^>]+id="__FRONTITY_CONNECT_STATE__"[^>]*>([\s\S]*?)<\/script>/i);

  if (!scriptMatch || !scriptMatch[1]) {
    return [];
  }

  let parsedPayload: unknown = null;
  try {
    parsedPayload = JSON.parse(scriptMatch[1]);
  } catch {
    return [];
  }

  const payloadRecord = asRecord(parsedPayload);
  const sourceRecord = asRecord(payloadRecord?.source);
  const sourceData = asRecord(sourceRecord?.data);

  if (!sourceData) {
    return [];
  }

  const entryKeys = Object.keys(sourceData);
  const mediaEntryKey = entryKeys.find((entryKey) => entryKey.startsWith('/embed/v2/'));

  if (!mediaEntryKey) {
    return [];
  }

  const mediaEntry = asRecord(sourceData[mediaEntryKey]);
  const videoData = asRecord(mediaEntry?.videoData);
  const itemInfos = asRecord(videoData?.itemInfos);

  if (!itemInfos) {
    return [];
  }

  const candidateUrls = new Set<string>();
  const addCandidate = (candidate: unknown) => {
    const normalized = asNonEmptyString(candidate);
    if (!normalized) {
      return;
    }

    if (normalized.startsWith('http://') || normalized.startsWith('https://')) {
      candidateUrls.add(normalized);
    }
  };

  const addFromStringArray = (candidate: unknown) => {
    if (!Array.isArray(candidate)) {
      return;
    }

    candidate.forEach((entry) => addCandidate(entry));
  };

  const videoRecord = asRecord(itemInfos.video);
  addFromStringArray(videoRecord?.urls);

  const imagePostRecord = asRecord(itemInfos.imagePostInfo) || asRecord(itemInfos.imagePost);
  const displayImages = Array.isArray(imagePostRecord?.displayImages) ? imagePostRecord?.displayImages : [];

  for (const imageEntry of displayImages) {
    const imageRecord = asRecord(imageEntry);
    if (!imageRecord) {
      continue;
    }
    addFromStringArray(imageRecord.urlList);
    addFromStringArray(asRecord(imageRecord.imageURL)?.urlList);
    addFromStringArray(asRecord(imageRecord.displayImage)?.urlList);
  }

  addFromStringArray(itemInfos.coversOrigin);
  addFromStringArray(itemInfos.covers);
  addFromStringArray(itemInfos.coversDynamic);

  return [...candidateUrls];
};

const downloadTikTokWithPageExtraction = async (sourceUrl: string, tmpDir: string): Promise<string> => {
  const pageResponse = await fetch(sourceUrl, {
    headers: {
      'user-agent': TIKTOK_PAGE_USER_AGENT,
      accept: 'text/html,application/xhtml+xml',
    },
  }).catch((error) => {
    throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
  });

  if (!pageResponse.ok) {
    throw new MediaIngestionError(
      getMediaErrorCodeFromHttpStatus(pageResponse.status),
      `Unable to download media (${pageResponse.status})`,
      `TikTok page download failed with status ${pageResponse.status} for ${sourceUrl}`,
    );
  }

  const pageHtml = await pageResponse.text().catch((error) => {
    throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
  });

  const mediaCandidates = new Set<string>(extractTikTokMediaUrlsFromHtml(pageHtml));
  const canonicalUrl = extractTikTokCanonicalUrlFromHtml(pageHtml);
  const candidateItemIds = new Set<string>();

  [sourceUrl, pageResponse.url, canonicalUrl].forEach((candidateUrl) => {
    const normalizedCandidateUrl = asNonEmptyString(candidateUrl);
    if (!normalizedCandidateUrl) {
      return;
    }

    const itemId = extractTikTokItemIdFromUrl(normalizedCandidateUrl);
    if (itemId) {
      candidateItemIds.add(itemId);
    }
  });

  for (const itemId of candidateItemIds) {
    const embedUrl = `https://www.tiktok.com/embed/v2/${itemId}`;
    try {
      const embedResponse = await fetch(embedUrl, {
        headers: {
          'user-agent': TIKTOK_PAGE_USER_AGENT,
          accept: 'text/html,application/xhtml+xml',
        },
      }).catch((error) => {
        throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
      });

      if (!embedResponse.ok) {
        continue;
      }

      const embedHtml = await embedResponse.text().catch(() => '');
      const embedCandidates = extractTikTokEmbedMediaUrlsFromHtml(embedHtml);
      embedCandidates.forEach((candidateUrl) => mediaCandidates.add(candidateUrl));
    } catch {
      // Keep existing candidates and continue trying other item ids.
    }
  }

  const resolvedMediaCandidates = [...mediaCandidates];

  if (resolvedMediaCandidates.length === 0) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unable to resolve TikTok media URL',
      `No TikTok media URL was found in page payload for ${sourceUrl}`,
    );
  }

  const cookieHeader = buildCookieHeader(extractTikTokSetCookieHeaders(pageResponse));

  let lastError: unknown = null;

  for (const mediaUrl of resolvedMediaCandidates) {
    try {
      const requestHeaders: Record<string, string> = {
        'user-agent': TIKTOK_PAGE_USER_AGENT,
        referer: sourceUrl,
      };

      if (cookieHeader) {
        requestHeaders.cookie = cookieHeader;
      }

      const mediaResponse = await fetch(mediaUrl, {
        headers: requestHeaders,
      }).catch((error) => {
        throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
      });

      if (!mediaResponse.ok) {
        throw new MediaIngestionError(
          getMediaErrorCodeFromHttpStatus(mediaResponse.status),
          `Unable to download media (${mediaResponse.status})`,
          `TikTok media request failed with status ${mediaResponse.status} for ${mediaUrl}`,
        );
      }

      const mediaContentType = (mediaResponse.headers.get('content-type') || '').toLowerCase();
      if (
        mediaContentType &&
        !mediaContentType.includes('video/') &&
        !mediaContentType.includes('image/') &&
        !mediaContentType.includes('application/octet-stream')
      ) {
        throw new MediaIngestionError(
          'DOWNLOAD_FAILED',
          'Unsupported TikTok media type',
          `TikTok candidate returned unsupported content-type "${mediaContentType}" for ${mediaUrl}`,
        );
      }

      return await downloadHttpResponseToTempFile({
        response: mediaResponse,
        tmpDir,
        sourceUrl,
        outputBasename: 'download-tiktok',
      });
    } catch (candidateError) {
      lastError = candidateError;
    }
  }

  if (lastError) {
    throw toMediaIngestionError(lastError, 'DOWNLOAD_FAILED');
  }

  throw new MediaIngestionError(
    'DOWNLOAD_FAILED',
    'Unable to download TikTok media',
    `TikTok media candidates could not be downloaded for ${sourceUrl}`,
  );
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

  return downloadHttpResponseToTempFile({
    response,
    tmpDir,
    sourceUrl,
    outputBasename: 'download-direct',
  });
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

  if (isTikTokUrl(sourceUrl)) {
    try {
      return await downloadTikTokWithPageExtraction(sourceUrl, tmpDir);
    } catch (tiktokError) {
      const normalizedTikTokError = toMediaIngestionError(tiktokError);
      if (normalizedTikTokError.code === 'FILE_TOO_LARGE') {
        throw normalizedTikTokError;
      }
      logger.warn(`[MEDIA] TikTok extraction fallback for ${sourceUrl} (${normalizedTikTokError.code})`);
    }
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
