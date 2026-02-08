export type MediaIngestionErrorCode =
  | 'UNSUPPORTED_SOURCE'
  | 'PRIVATE_OR_AUTH_REQUIRED'
  | 'DRM_PROTECTED'
  | 'MEDIA_NOT_FOUND'
  | 'DOWNLOAD_TIMEOUT'
  | 'FILE_TOO_LARGE'
  | 'INVALID_MEDIA'
  | 'TRANSCODE_FAILED'
  | 'DOWNLOAD_FAILED';

const ERROR_PRIORITY: Record<MediaIngestionErrorCode, number> = {
  UNSUPPORTED_SOURCE: 90,
  PRIVATE_OR_AUTH_REQUIRED: 85,
  DRM_PROTECTED: 80,
  MEDIA_NOT_FOUND: 75,
  FILE_TOO_LARGE: 70,
  INVALID_MEDIA: 65,
  TRANSCODE_FAILED: 60,
  DOWNLOAD_TIMEOUT: 55,
  DOWNLOAD_FAILED: 50,
};

export class MediaIngestionError extends Error {
  public readonly code: MediaIngestionErrorCode;
  public readonly rawMessage: string;

  constructor(code: MediaIngestionErrorCode, message: string, rawMessage?: string) {
    super(message);
    this.name = 'MediaIngestionError';
    this.code = code;
    this.rawMessage = rawMessage || message;
  }
}

const asRecord = (value: unknown): Record<string, unknown> | null => {
  if (!value || typeof value !== 'object') {
    return null;
  }

  return value as Record<string, unknown>;
};

const safeString = (value: unknown): string => {
  if (typeof value === 'string') {
    return value;
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }

  return '';
};

const buildRawErrorText = (error: unknown) => {
  const chunks: string[] = [];

  if (error instanceof Error) {
    chunks.push(error.message || '');
  }

  const record = asRecord(error);

  if (record) {
    chunks.push(safeString(record.stderr));
    chunks.push(safeString(record.stdout));
    chunks.push(safeString(record.code));
  }

  const fallbackText = safeString(error);

  if (fallbackText && chunks.length === 0) {
    chunks.push(fallbackText);
  }

  return chunks
    .filter((chunk) => chunk.trim() !== '')
    .join('\n')
    .slice(0, 2000);
};

const extractHttpStatus = (rawText: string): number | null => {
  const matches = rawText.match(/\b([45]\d{2})\b/g);

  if (!matches || matches.length === 0) {
    return null;
  }

  const status = parseInt(matches[0], 10);

  return Number.isFinite(status) ? status : null;
};

export const getMediaErrorCodeFromHttpStatus = (status: number): MediaIngestionErrorCode => {
  if (status === 401 || status === 403) {
    return 'PRIVATE_OR_AUTH_REQUIRED';
  }

  if (status === 404 || status === 410) {
    return 'MEDIA_NOT_FOUND';
  }

  if (status === 408 || status === 504) {
    return 'DOWNLOAD_TIMEOUT';
  }

  if (status === 413) {
    return 'FILE_TOO_LARGE';
  }

  return 'DOWNLOAD_FAILED';
};

const classifyMediaErrorCode = (rawText: string, fallbackCode: MediaIngestionErrorCode): MediaIngestionErrorCode => {
  const text = rawText.toLowerCase();

  const statusCode = extractHttpStatus(text);
  if (statusCode !== null) {
    return getMediaErrorCodeFromHttpStatus(statusCode);
  }

  if (
    text.includes('unsupported url') ||
    text.includes('unsupported site') ||
    text.includes('no suitable extractor') ||
    text.includes('unsupported webpage')
  ) {
    return 'UNSUPPORTED_SOURCE';
  }

  if (text.includes('drm')) {
    return 'DRM_PROTECTED';
  }

  if (
    text.includes('private') ||
    text.includes('login required') ||
    text.includes('sign in') ||
    text.includes('age-restricted') ||
    text.includes('members only') ||
    text.includes('forbidden')
  ) {
    return 'PRIVATE_OR_AUTH_REQUIRED';
  }

  if (text.includes('not found') || text.includes('video unavailable') || text.includes('unavailable')) {
    return 'MEDIA_NOT_FOUND';
  }

  if (
    text.includes('timed out') ||
    text.includes('timeout') ||
    text.includes('etimedout') ||
    text.includes('econnreset') ||
    text.includes('network is unreachable')
  ) {
    return 'DOWNLOAD_TIMEOUT';
  }

  if (text.includes('too large') || text.includes('payload too large') || text.includes('content too large')) {
    return 'FILE_TOO_LARGE';
  }

  if (
    text.includes('invalid data found when processing input') ||
    text.includes('moov atom not found') ||
    text.includes('invalid or corrupted') ||
    text.includes('could not find codec parameters')
  ) {
    return 'INVALID_MEDIA';
  }

  if (text.includes('ffmpeg') || text.includes('ffprobe') || text.includes('conversion failed')) {
    return 'TRANSCODE_FAILED';
  }

  if (text.includes('unable to download') || text.includes('http error')) {
    return 'DOWNLOAD_FAILED';
  }

  return fallbackCode;
};

export const toMediaIngestionError = (
  error: unknown,
  fallbackCode: MediaIngestionErrorCode = 'DOWNLOAD_FAILED',
): MediaIngestionError => {
  if (error instanceof MediaIngestionError) {
    return error;
  }

  const rawText = buildRawErrorText(error);
  const code = classifyMediaErrorCode(rawText, fallbackCode);

  const message = rawText.split('\n').map((line) => line.trim()).find(Boolean) || 'Media ingestion failed';

  return new MediaIngestionError(code, message, rawText);
};

export const pickMostRelevantMediaError = (first: unknown, second: unknown): MediaIngestionError => {
  const firstError = toMediaIngestionError(first);
  const secondError = toMediaIngestionError(second);

  if (ERROR_PRIORITY[firstError.code] >= ERROR_PRIORITY[secondError.code]) {
    return firstError;
  }

  return secondError;
};

export const getLocalizedMediaErrorMessage = (error: unknown): string => {
  const normalized = toMediaIngestionError(error);

  switch (normalized.code) {
    case 'UNSUPPORTED_SOURCE':
      return rosetty.t('sendCommandMediaErrorUnsupportedSource')!;
    case 'PRIVATE_OR_AUTH_REQUIRED':
      return rosetty.t('sendCommandMediaErrorPrivate')!;
    case 'DRM_PROTECTED':
      return rosetty.t('sendCommandMediaErrorDrm')!;
    case 'MEDIA_NOT_FOUND':
      return rosetty.t('sendCommandMediaErrorNotFound')!;
    case 'DOWNLOAD_TIMEOUT':
      return rosetty.t('sendCommandMediaErrorTimeout')!;
    case 'FILE_TOO_LARGE':
      return rosetty.t('sendCommandMediaErrorTooLarge')!;
    case 'INVALID_MEDIA':
      return rosetty.t('sendCommandMediaErrorInvalidMedia')!;
    case 'TRANSCODE_FAILED':
      return rosetty.t('sendCommandMediaErrorTranscode')!;
    case 'DOWNLOAD_FAILED':
    default:
      return rosetty.t('sendCommandMediaErrorDownload')!;
  }
};
