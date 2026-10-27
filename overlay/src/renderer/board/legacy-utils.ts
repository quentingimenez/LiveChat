export interface BoardLegacyUtils {
  normalizeMediaKind(value: unknown): 'video' | 'image' | 'audio';
  toSafeDateLabel(value: unknown): string;
  toCardTitle(item: unknown): string;
  toMessagePreview(value: unknown, maxLength?: number): string;
  clamp01(value: unknown): number;
  toPerceptualGain(linearVolume: unknown, gamma?: number): number;
  isHttpUrl(value: unknown): boolean;
}

export function createBoardLegacyUtils(): BoardLegacyUtils {
  function normalizeMediaKind(value: unknown): 'video' | 'image' | 'audio' {
    const normalized = `${value || ''}`.trim().toLowerCase();

    if (normalized === 'image' || normalized === 'audio' || normalized === 'video') {
      return normalized;
    }

    return 'video';
  }

  function toSafeDateLabel(value: unknown): string {
    const parsed = new Date(value as string);

    if (Number.isNaN(parsed.getTime())) {
      return 'Date inconnue';
    }

    return parsed.toLocaleString('fr-FR');
  }

  function toCardTitle(item: unknown): string {
    const record = typeof item === 'object' && item !== null ? (item as Record<string, unknown>) : {};
    const rawTitle = `${record.title || ''}`.trim();

    if (rawTitle) {
      return rawTitle;
    }

    const mediaRecord =
      typeof record.media === 'object' && record.media !== null ? (record.media as Record<string, unknown>) : {};
    const source = `${mediaRecord.sourceUrl || ''}`.trim();

    if (!source) {
      return `${record.mediaAssetId || 'Media'}`;
    }

    try {
      const url = new URL(source);
      return `${url.hostname}${url.pathname}`;
    } catch {
      return source;
    }
  }

  function toMessagePreview(value: unknown, maxLength = 120): string {
    const normalized = `${value || ''}`.trim().replace(/\s+/g, ' ');

    if (!normalized) {
      return '';
    }

    if (normalized.length <= maxLength) {
      return normalized;
    }

    return `${normalized.slice(0, Math.max(1, maxLength - 1))}…`;
  }

  function clamp01(value: unknown): number {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      return 1;
    }

    return Math.min(1, Math.max(0, value));
  }

  function toPerceptualGain(linearVolume: unknown, gamma = 2.2): number {
    const normalized = clamp01(linearVolume);
    return Math.pow(normalized, gamma);
  }

  function isHttpUrl(value: unknown): boolean {
    const raw = `${value || ''}`.trim();
    if (!raw) {
      return false;
    }

    try {
      const parsed = new URL(raw);
      return parsed.protocol === 'https:' || parsed.protocol === 'http:';
    } catch {
      return false;
    }
  }

  return {
    normalizeMediaKind,
    toSafeDateLabel,
    toCardTitle,
    toMessagePreview,
    clamp01,
    toPerceptualGain,
    isHttpUrl
  };
}
