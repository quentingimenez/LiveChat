const BYTES_PER_MEGABYTE = 1024 * 1024;
const BYTES_PER_GIGABYTE = 1024 * BYTES_PER_MEGABYTE;

export const toNonEmptyString = (value: unknown): string | null => {
  if (typeof value !== 'string') {
    return null;
  }

  const normalized = value.trim();
  return normalized.length > 0 ? normalized : null;
};

export const toBooleanFlag = (value: unknown): boolean => {
  if (typeof value === 'boolean') {
    return value;
  }

  if (typeof value === 'number') {
    return value === 1;
  }

  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase();
    return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
  }

  return false;
};

export const formatBytesForMessage = (value: number): string => {
  const normalized = Number.isFinite(value) && value > 0 ? value : 0;

  if (normalized >= BYTES_PER_GIGABYTE) {
    return `${(normalized / BYTES_PER_GIGABYTE).toFixed(2)} GB`;
  }

  if (normalized === 0) {
    return '0 MB';
  }

  return `${(normalized / BYTES_PER_MEGABYTE).toFixed(2)} MB`;
};
