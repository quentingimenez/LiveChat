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
const TIKTOK_CAROUSEL_IMAGE_DURATION_SEC = 3;
const TIKTOK_MAX_CAROUSEL_IMAGES = 35;
const TIKTOK_MAX_CAROUSEL_CANDIDATES_PER_SLIDE = 3;
const TIKTOK_CAROUSEL_FPS = 30;

interface TikTokEmbedMediaCandidates {
  videoUrls: string[];
  imageUrls: string[];
  audioUrls: string[];
  photoSlides: string[][];
}

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

const sanitizeUrlForLog = (rawUrl: string): string => {
  const normalized = rawUrl.trim();
  if (!normalized) {
    return normalized;
  }

  try {
    const parsed = new URL(normalized);
    parsed.search = '';
    parsed.hash = '';
    return parsed.toString();
  } catch {
    return normalized.slice(0, 240);
  }
};

const toEvenNumber = (value: number): number => {
  const rounded = Math.max(2, Math.round(value));
  return rounded % 2 === 0 ? rounded : rounded - 1;
};

const decodeTikTokEscapedUrl = (value: string): string => {
  return value
    .replace(/\\u0026/gi, '&')
    .replace(/&amp;/gi, '&')
    .replace(/\\\//g, '/');
};

const extractTikTokPhotoAssetKey = (mediaUrl: string): string | null => {
  try {
    const parsed = new URL(mediaUrl);
    const match = parsed.pathname.match(/\/([^/?#]+)~tplv-photomode-image\./i);
    return asNonEmptyString(match?.[1]);
  } catch {
    return null;
  }
};

const buildTikTokPhotoSlidesFromUrls = (urls: string[]): string[][] => {
  const groupedSlides = new Map<string, string[]>();

  for (const mediaUrl of urls) {
    const groupKey = extractTikTokPhotoAssetKey(mediaUrl) || mediaUrl;
    const existingUrls = groupedSlides.get(groupKey);

    if (!existingUrls) {
      groupedSlides.set(groupKey, [mediaUrl]);
      continue;
    }

    if (!existingUrls.includes(mediaUrl)) {
      existingUrls.push(mediaUrl);
    }
  }

  return [...groupedSlides.values()].filter((slide) => slide.length > 0);
};

const extractTikTokPhotoUrlsFromRawHtml = (html: string): string[] => {
  const escapedMatches =
    html.match(/https?:\\\/\\\/[^"'<>\s]+~tplv-photomode-image\.(?:jpeg|jpg|png)[^"'<>\s]*/gi) || [];
  const plainMatches = html.match(/https?:\/\/[^"'<>\s]+~tplv-photomode-image\.(?:jpeg|jpg|png)[^"'<>\s]*/gi) || [];
  const dedupeSet = new Set<string>();
  const urls: string[] = [];

  for (const rawCandidate of [...escapedMatches, ...plainMatches]) {
    const decodedCandidate = decodeTikTokEscapedUrl(rawCandidate);
    const normalized = asNonEmptyString(decodedCandidate);

    if (!normalized) {
      continue;
    }

    if (!normalized.startsWith('http://') && !normalized.startsWith('https://')) {
      continue;
    }

    if (!dedupeSet.has(normalized)) {
      dedupeSet.add(normalized);
      urls.push(normalized);
    }
  }

  return urls;
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

const extractTikTokEmbedMediaCandidatesFromHtml = (html: string): TikTokEmbedMediaCandidates => {
  const scriptMatch = html.match(/<script[^>]+id="__FRONTITY_CONNECT_STATE__"[^>]*>([\s\S]*?)<\/script>/i);

  if (!scriptMatch || !scriptMatch[1]) {
    return {
      videoUrls: [],
      imageUrls: [],
      audioUrls: [],
      photoSlides: [],
    };
  }

  let parsedPayload: unknown = null;
  try {
    parsedPayload = JSON.parse(scriptMatch[1]);
  } catch {
    return {
      videoUrls: [],
      imageUrls: [],
      audioUrls: [],
      photoSlides: [],
    };
  }

  const payloadRecord = asRecord(parsedPayload);
  const sourceRecord = asRecord(payloadRecord?.source);
  const sourceData = asRecord(sourceRecord?.data);

  if (!sourceData) {
    return {
      videoUrls: [],
      imageUrls: [],
      audioUrls: [],
      photoSlides: [],
    };
  }

  const entryKeys = Object.keys(sourceData);
  const mediaEntryKey = entryKeys.find((entryKey) => entryKey.startsWith('/embed/v2/'));

  if (!mediaEntryKey) {
    return {
      videoUrls: [],
      imageUrls: [],
      audioUrls: [],
      photoSlides: [],
    };
  }

  const mediaEntry = asRecord(sourceData[mediaEntryKey]);
  const videoData = asRecord(mediaEntry?.videoData);
  const itemInfos = asRecord(videoData?.itemInfos);

  if (!itemInfos) {
    return {
      videoUrls: [],
      imageUrls: [],
      audioUrls: [],
      photoSlides: [],
    };
  }

  const videoUrls: string[] = [];
  const videoUrlsSet = new Set<string>();
  const imageUrls: string[] = [];
  const imageUrlsSet = new Set<string>();
  const audioUrls: string[] = [];
  const audioUrlsSet = new Set<string>();
  const photoSlides: string[][] = [];

  const appendUrl = (target: string[], dedupeSet: Set<string>, candidate: unknown) => {
    const normalized = asNonEmptyString(candidate);
    if (!normalized) {
      return;
    }

    if (normalized.startsWith('http://') || normalized.startsWith('https://')) {
      if (!dedupeSet.has(normalized)) {
        dedupeSet.add(normalized);
        target.push(normalized);
      }
    }
  };

  const appendFromStringArray = (target: string[], dedupeSet: Set<string>, candidate: unknown) => {
    if (!Array.isArray(candidate)) {
      return;
    }

    candidate.forEach((entry) => appendUrl(target, dedupeSet, entry));
  };

  const videoRecord = asRecord(itemInfos.video);
  appendFromStringArray(videoUrls, videoUrlsSet, videoRecord?.urls);
  appendFromStringArray(videoUrls, videoUrlsSet, asRecord(videoRecord?.playAddr)?.urlList);

  const videoMusicInfos = asRecord(videoData?.musicInfos);
  const itemMusicInfos = asRecord(itemInfos.musicInfos);
  appendFromStringArray(audioUrls, audioUrlsSet, videoMusicInfos?.playUrl);
  appendFromStringArray(audioUrls, audioUrlsSet, itemMusicInfos?.playUrl);

  const imagePostRecord = asRecord(itemInfos.imagePostInfo) || asRecord(itemInfos.imagePost);
  const displayImages = Array.isArray(imagePostRecord?.displayImages) ? imagePostRecord?.displayImages : [];

  for (const imageEntry of displayImages) {
    const imageRecord = asRecord(imageEntry);
    if (!imageRecord) {
      continue;
    }

    const slideUrls: string[] = [];
    const slideUrlsSet = new Set<string>();
    appendFromStringArray(slideUrls, slideUrlsSet, imageRecord.urlList);
    appendFromStringArray(slideUrls, slideUrlsSet, asRecord(imageRecord.imageURL)?.urlList);
    appendFromStringArray(slideUrls, slideUrlsSet, asRecord(imageRecord.displayImage)?.urlList);

    if (slideUrls.length > 0) {
      photoSlides.push(slideUrls);
      slideUrls.forEach((slideUrl) => appendUrl(imageUrls, imageUrlsSet, slideUrl));
    }
  }

  appendFromStringArray(imageUrls, imageUrlsSet, itemInfos.coversOrigin);
  appendFromStringArray(imageUrls, imageUrlsSet, itemInfos.covers);
  appendFromStringArray(imageUrls, imageUrlsSet, itemInfos.coversDynamic);

  const photoUrlsFromRawHtml = extractTikTokPhotoUrlsFromRawHtml(html);
  photoUrlsFromRawHtml.forEach((photoUrl) => appendUrl(imageUrls, imageUrlsSet, photoUrl));

  if (photoSlides.length <= 1 && photoUrlsFromRawHtml.length > 1) {
    const groupedSlides = buildTikTokPhotoSlidesFromUrls(photoUrlsFromRawHtml);
    if (groupedSlides.length > photoSlides.length) {
      photoSlides.length = 0;
      groupedSlides.forEach((slide) => photoSlides.push(slide));
    }
  }

  return {
    videoUrls,
    imageUrls,
    audioUrls,
    photoSlides,
  };
};

const isSupportedTikTokContentType = (contentType: string): boolean => {
  const normalized = (contentType || '').toLowerCase();

  return (
    !normalized ||
    normalized.includes('video/') ||
    normalized.includes('image/') ||
    normalized.includes('audio/') ||
    normalized.includes('application/octet-stream')
  );
};

const downloadTikTokMediaCandidate = async (params: {
  sourceUrl: string;
  mediaUrl: string;
  tmpDir: string;
  cookieHeader: string;
  outputBasename: string;
  expectedKind: 'video' | 'image' | 'audio' | 'any';
}) => {
  const requestHeaders: Record<string, string> = {
    'user-agent': TIKTOK_PAGE_USER_AGENT,
    referer: params.sourceUrl,
    accept: 'video/*,image/*,audio/*,*/*;q=0.8',
  };

  if (params.cookieHeader) {
    requestHeaders.cookie = params.cookieHeader;
  }

  const mediaResponse = await fetch(params.mediaUrl, {
    headers: requestHeaders,
  }).catch((error) => {
    throw toMediaIngestionError(error, 'DOWNLOAD_FAILED');
  });

  if (!mediaResponse.ok) {
    throw new MediaIngestionError(
      getMediaErrorCodeFromHttpStatus(mediaResponse.status),
      `Unable to download media (${mediaResponse.status})`,
      `TikTok media request failed with status ${mediaResponse.status} for ${params.mediaUrl}`,
    );
  }

  const mediaContentType = (mediaResponse.headers.get('content-type') || '').toLowerCase();

  if (!isSupportedTikTokContentType(mediaContentType)) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unsupported TikTok media type',
      `TikTok candidate returned unsupported content-type "${mediaContentType}" for ${params.mediaUrl}`,
    );
  }

  if (
    params.expectedKind === 'video' &&
    mediaContentType &&
    !mediaContentType.includes('video/') &&
    !mediaContentType.includes('application/octet-stream')
  ) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unexpected TikTok media type',
      `Expected video content-type for ${params.mediaUrl}, got "${mediaContentType}"`,
    );
  }

  if (
    params.expectedKind === 'image' &&
    mediaContentType &&
    !mediaContentType.includes('image/') &&
    !mediaContentType.includes('application/octet-stream')
  ) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unexpected TikTok media type',
      `Expected image content-type for ${params.mediaUrl}, got "${mediaContentType}"`,
    );
  }

  if (
    params.expectedKind === 'audio' &&
    mediaContentType &&
    !mediaContentType.includes('audio/') &&
    !mediaContentType.includes('application/octet-stream')
  ) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unexpected TikTok media type',
      `Expected audio content-type for ${params.mediaUrl}, got "${mediaContentType}"`,
    );
  }

  return downloadHttpResponseToTempFile({
    response: mediaResponse,
    tmpDir: params.tmpDir,
    sourceUrl: params.sourceUrl,
    outputBasename: params.outputBasename,
  });
};

const downloadTikTokCarouselSlides = async (params: {
  sourceUrl: string;
  tmpDir: string;
  cookieHeader: string;
  photoSlides: string[][];
}) => {
  const downloadedSlides: string[] = [];

  for (const [slideIndex, slideCandidates] of params.photoSlides.entries()) {
    let downloadedSlidePath: string | null = null;
    let lastSlideError: unknown = null;

    for (const [candidateIndex, candidateUrl] of slideCandidates.entries()) {
      try {
        downloadedSlidePath = await downloadTikTokMediaCandidate({
          sourceUrl: params.sourceUrl,
          mediaUrl: candidateUrl,
          tmpDir: params.tmpDir,
          cookieHeader: params.cookieHeader,
          outputBasename: `download-tiktok-slide-${String(slideIndex + 1).padStart(2, '0')}-${candidateIndex + 1}`,
          expectedKind: 'image',
        });
        break;
      } catch (slideCandidateError) {
        lastSlideError = slideCandidateError;
        const normalized = toMediaIngestionError(slideCandidateError, 'DOWNLOAD_FAILED');
        logger.warn(
          {
            sourceUrl: sanitizeUrlForLog(params.sourceUrl),
            slideIndex: slideIndex + 1,
            candidateIndex: candidateIndex + 1,
            candidateUrl: sanitizeUrlForLog(candidateUrl),
            code: normalized.code,
          },
          '[MEDIA] TikTok carousel slide candidate failed',
        );
      }
    }

    if (!downloadedSlidePath) {
      if (lastSlideError) {
        throw lastSlideError;
      }

      throw new MediaIngestionError(
        'DOWNLOAD_FAILED',
        'Unable to download TikTok carousel slide',
        `No valid image candidate for TikTok slide #${slideIndex + 1} (${params.sourceUrl})`,
      );
    }

    downloadedSlides.push(downloadedSlidePath);
  }

  return downloadedSlides;
};

const downloadTikTokCarouselAudio = async (params: {
  sourceUrl: string;
  tmpDir: string;
  cookieHeader: string;
  audioCandidates: string[];
}) => {
  let lastAudioError: unknown = null;

  for (const [candidateIndex, candidateUrl] of params.audioCandidates.entries()) {
    try {
      return await downloadTikTokMediaCandidate({
        sourceUrl: params.sourceUrl,
        mediaUrl: candidateUrl,
        tmpDir: params.tmpDir,
        cookieHeader: params.cookieHeader,
        outputBasename: `download-tiktok-audio-${candidateIndex + 1}`,
        expectedKind: 'audio',
      });
    } catch (audioCandidateError) {
      lastAudioError = audioCandidateError;
      const normalized = toMediaIngestionError(audioCandidateError, 'DOWNLOAD_FAILED');
      logger.warn(
        {
          sourceUrl: sanitizeUrlForLog(params.sourceUrl),
          candidateIndex: candidateIndex + 1,
          totalCandidates: params.audioCandidates.length,
          candidateUrl: sanitizeUrlForLog(candidateUrl),
          code: normalized.code,
        },
        '[MEDIA] TikTok carousel audio candidate failed',
      );
    }
  }

  if (lastAudioError) {
    throw lastAudioError;
  }

  return null;
};

const probeMediaDimensionsFromFile = async (filePath: string): Promise<{ width: number; height: number }> => {
  const { stdout } = await execFileAsync(
    env.FFPROBE_BINARY,
    ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0:s=x', filePath],
    {
      timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
      maxBuffer: 1024 * 1024,
    },
  );

  const dimensionsText = (stdout || '').trim();
  const match = dimensionsText.match(/^(\d+)x(\d+)$/);

  if (!match) {
    throw new MediaIngestionError(
      'INVALID_MEDIA',
      'Unable to parse media dimensions',
      `ffprobe returned "${dimensionsText}" for ${filePath}`,
    );
  }

  const width = Number.parseInt(match[1] || '0', 10);
  const height = Number.parseInt(match[2] || '0', 10);

  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    throw new MediaIngestionError(
      'INVALID_MEDIA',
      'Invalid media dimensions',
      `Invalid ffprobe dimensions "${dimensionsText}" for ${filePath}`,
    );
  }

  return {
    width,
    height,
  };
};

const createTikTokCarouselVideo = async (params: {
  sourceUrl: string;
  tmpDir: string;
  imagePaths: string[];
  audioPath: string | null;
}) => {
  if (params.imagePaths.length < 2) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unable to build TikTok carousel',
      `Expected at least 2 images to build carousel for ${params.sourceUrl}`,
    );
  }

  const outputVideoPath = path.join(params.tmpDir, 'download-tiktok-carousel.mp4');
  const firstImageDimensions = await probeMediaDimensionsFromFile(params.imagePaths[0]);
  const configuredMaxHeight = env.MEDIA_VIDEO_MAX_HEIGHT > 0 ? env.MEDIA_VIDEO_MAX_HEIGHT : 1080;
  const portraitWidth = toEvenNumber((configuredMaxHeight * 9) / 16);
  const landscapeHeight = toEvenNumber((configuredMaxHeight * 9) / 16);
  const isPortrait = firstImageDimensions.height >= firstImageDimensions.width;
  const targetWidth = isPortrait ? portraitWidth : toEvenNumber(configuredMaxHeight);
  const targetHeight = isPortrait ? toEvenNumber(configuredMaxHeight) : landscapeHeight;
  const configuredPreset = (env.MEDIA_VIDEO_PRESET || '').trim();
  const selectedPreset = configuredPreset && configuredPreset !== 'superfast' ? configuredPreset : 'ultrafast';
  const inputArgs: string[] = [];
  const filterParts: string[] = [];

  params.imagePaths.forEach((imagePath, index) => {
    inputArgs.push('-loop', '1', '-t', `${TIKTOK_CAROUSEL_IMAGE_DURATION_SEC}`, '-i', imagePath);
    filterParts.push(
      `[${index}:v]fps=${TIKTOK_CAROUSEL_FPS},scale=${targetWidth}:${targetHeight}:force_original_aspect_ratio=decrease,pad=${targetWidth}:${targetHeight}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p[v${index}]`,
    );
  });

  const concatenatedInputs = params.imagePaths.map((_, index) => `[v${index}]`).join('');
  filterParts.push(`${concatenatedInputs}concat=n=${params.imagePaths.length}:v=1:a=0[vout]`);

  if (params.audioPath) {
    inputArgs.push('-i', params.audioPath);
  }

  const ffmpegArgs = [
    '-y',
    ...inputArgs,
    '-filter_complex',
    filterParts.join(';'),
    '-map',
    '[vout]',
    '-c:v',
    'libx264',
    '-preset',
    selectedPreset,
    '-pix_fmt',
    'yuv420p',
    '-movflags',
    '+faststart',
  ];

  if (params.audioPath) {
    const audioInputIndex = params.imagePaths.length;
    ffmpegArgs.push('-map', `${audioInputIndex}:a:0?`, '-c:a', 'aac', '-b:a', '192k', '-shortest');
  } else {
    ffmpegArgs.push('-an');
  }

  ffmpegArgs.push(outputVideoPath);

  await execFileAsync(env.FFMPEG_BINARY, ffmpegArgs, {
    timeout: env.MEDIA_DOWNLOAD_TIMEOUT_MS,
    maxBuffer: 10 * 1024 * 1024,
  });

  await ensureFileSizeWithinLimit(outputVideoPath, params.sourceUrl);

  return outputVideoPath;
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

  const videoCandidates = new Set<string>(extractTikTokMediaUrlsFromHtml(pageHtml));
  const imageCandidates = new Set<string>();
  const audioCandidates = new Set<string>();
  const canonicalUrl = extractTikTokCanonicalUrlFromHtml(pageHtml);
  const candidateItemIds = new Set<string>();
  let photoSlides: string[][] = [];

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
      const embedCandidates = extractTikTokEmbedMediaCandidatesFromHtml(embedHtml);
      embedCandidates.videoUrls.forEach((candidateUrl) => videoCandidates.add(candidateUrl));
      embedCandidates.imageUrls.forEach((candidateUrl) => imageCandidates.add(candidateUrl));
      embedCandidates.audioUrls.forEach((candidateUrl) => audioCandidates.add(candidateUrl));

      if (embedCandidates.photoSlides.length > photoSlides.length) {
        photoSlides = embedCandidates.photoSlides;
      }
    } catch {
      // Keep existing candidates and continue trying other item ids.
    }
  }

  logger.info(
    {
      sourceUrl: sanitizeUrlForLog(sourceUrl),
      resolvedUrl: sanitizeUrlForLog(pageResponse.url),
      itemIds: [...candidateItemIds],
      videoCandidates: videoCandidates.size,
      imageCandidates: imageCandidates.size,
      audioCandidates: audioCandidates.size,
      photoSlides: photoSlides.length,
    },
    '[MEDIA] TikTok candidates extracted',
  );

  if (videoCandidates.size === 0 && imageCandidates.size === 0 && audioCandidates.size === 0) {
    throw new MediaIngestionError(
      'DOWNLOAD_FAILED',
      'Unable to resolve TikTok media URL',
      `No TikTok media URL was found in page payload for ${sourceUrl}`,
    );
  }

  const cookieHeader = buildCookieHeader(extractTikTokSetCookieHeaders(pageResponse));

  let lastError: unknown = null;

  const resolvedPhotoSlides = photoSlides
    .slice(0, TIKTOK_MAX_CAROUSEL_IMAGES)
    .map((slide) =>
      slide
        .filter((slideUrl) => slideUrl.startsWith('http://') || slideUrl.startsWith('https://'))
        .slice(0, TIKTOK_MAX_CAROUSEL_CANDIDATES_PER_SLIDE),
    )
    .filter((slide) => slide.length > 0);
  const resolvedAudioCandidates = [...audioCandidates]
    .filter((audioUrl) => audioUrl.startsWith('http://') || audioUrl.startsWith('https://'))
    .slice(0, 5);

  if (resolvedPhotoSlides.length > 1) {
    try {
      const downloadedSlides = await downloadTikTokCarouselSlides({
        sourceUrl,
        tmpDir,
        cookieHeader,
        photoSlides: resolvedPhotoSlides,
      });
      let downloadedAudioPath: string | null = null;

      if (resolvedAudioCandidates.length > 0) {
        try {
          downloadedAudioPath = await downloadTikTokCarouselAudio({
            sourceUrl,
            tmpDir,
            cookieHeader,
            audioCandidates: resolvedAudioCandidates,
          });
        } catch (audioError) {
          const normalizedAudioError = toMediaIngestionError(audioError, 'DOWNLOAD_FAILED');
          logger.warn(
            {
              sourceUrl: sanitizeUrlForLog(sourceUrl),
              code: normalizedAudioError.code,
              message: normalizedAudioError.message,
            },
            '[MEDIA] TikTok carousel audio download failed, continuing without audio',
          );
        }
      }

      const carouselVideoPath = await createTikTokCarouselVideo({
        sourceUrl,
        tmpDir,
        imagePaths: downloadedSlides,
        audioPath: downloadedAudioPath,
      });

      logger.info(
        {
          sourceUrl: sanitizeUrlForLog(sourceUrl),
          slideCount: downloadedSlides.length,
          hasAudio: !!downloadedAudioPath,
        },
        '[MEDIA] TikTok carousel converted to slideshow video',
      );

      return carouselVideoPath;
    } catch (carouselError) {
      lastError = carouselError;
      const normalized = toMediaIngestionError(carouselError, 'DOWNLOAD_FAILED');
      logger.warn(
        {
          sourceUrl: sanitizeUrlForLog(sourceUrl),
          slideCount: resolvedPhotoSlides.length,
          code: normalized.code,
          message: normalized.message,
        },
        '[MEDIA] TikTok carousel conversion failed, fallback to single media candidate',
      );
    }
  }

  const resolvedMediaCandidates: Array<{
    mediaUrl: string;
    kind: 'video' | 'image' | 'audio';
  }> = [];
  const seenCandidateUrls = new Set<string>();
  const appendCandidate = (candidateUrl: string, kind: 'video' | 'image' | 'audio') => {
    if (seenCandidateUrls.has(candidateUrl)) {
      return;
    }

    seenCandidateUrls.add(candidateUrl);
    resolvedMediaCandidates.push({
      mediaUrl: candidateUrl,
      kind,
    });
  };

  videoCandidates.forEach((candidateUrl) => appendCandidate(candidateUrl, 'video'));
  imageCandidates.forEach((candidateUrl) => appendCandidate(candidateUrl, 'image'));
  audioCandidates.forEach((candidateUrl) => appendCandidate(candidateUrl, 'audio'));

  for (const [candidateIndex, candidate] of resolvedMediaCandidates.entries()) {
    try {
      const downloadedPath = await downloadTikTokMediaCandidate({
        sourceUrl,
        mediaUrl: candidate.mediaUrl,
        tmpDir,
        cookieHeader,
        outputBasename: 'download-tiktok',
        expectedKind: candidate.kind,
      });

      logger.info(
        {
          sourceUrl: sanitizeUrlForLog(sourceUrl),
          candidateIndex: candidateIndex + 1,
          totalCandidates: resolvedMediaCandidates.length,
          candidateKind: candidate.kind,
          candidateUrl: sanitizeUrlForLog(candidate.mediaUrl),
        },
        '[MEDIA] TikTok media candidate selected',
      );

      return downloadedPath;
    } catch (candidateError) {
      lastError = candidateError;
      const normalized = toMediaIngestionError(candidateError, 'DOWNLOAD_FAILED');
      logger.warn(
        {
          sourceUrl: sanitizeUrlForLog(sourceUrl),
          candidateIndex: candidateIndex + 1,
          totalCandidates: resolvedMediaCandidates.length,
          candidateKind: candidate.kind,
          candidateUrl: sanitizeUrlForLog(candidate.mediaUrl),
          code: normalized.code,
          message: normalized.message,
        },
        '[MEDIA] TikTok media candidate failed',
      );
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
    logger.warn(
      {
        sourceUrl: sanitizeUrlForLog(sourceUrl),
        code: normalized.code,
        message: normalized.message,
      },
      '[MEDIA] yt-dlp fallback',
    );
  }

  if (isTikTokUrl(sourceUrl)) {
    try {
      return await downloadTikTokWithPageExtraction(sourceUrl, tmpDir);
    } catch (tiktokError) {
      const normalizedTikTokError = toMediaIngestionError(tiktokError);
      if (normalizedTikTokError.code === 'FILE_TOO_LARGE') {
        throw normalizedTikTokError;
      }
      logger.warn(
        {
          sourceUrl: sanitizeUrlForLog(sourceUrl),
          code: normalizedTikTokError.code,
          message: normalizedTikTokError.message,
        },
        '[MEDIA] TikTok extraction fallback',
      );
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

  const normalized = await normalizeDownloadedMedia(params.inputFilePath, outputBasePath).catch((error) => {
    throw toMediaIngestionError(error, 'TRANSCODE_FAILED');
  });

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
