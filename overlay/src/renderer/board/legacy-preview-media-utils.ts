interface BuildAuthedUrlParams {
  config: unknown;
  pathname: string;
  options?: Record<string, unknown>;
}

interface ApplyPreviewVolumeParams {
  root: unknown;
  linearVolume: unknown;
  gamma: number;
}

export interface BoardLegacyPreviewMediaUtils {
  buildAuthedUrl(params: BuildAuthedUrlParams): URL;
  applyPreviewVolume(params: ApplyPreviewVolumeParams): void;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function toFiniteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function clamp01(value: unknown): number {
  const numeric = toFiniteNumber(value, 1);
  if (numeric <= 0) {
    return 0;
  }

  if (numeric >= 1) {
    return 1;
  }

  return numeric;
}

function toPerceptualGain(linearVolume: unknown, gamma: number): number {
  return Math.pow(clamp01(linearVolume), toFiniteNumber(gamma, 2.2));
}

export function createBoardLegacyPreviewMediaUtils(): BoardLegacyPreviewMediaUtils {
  function buildAuthedUrl(params: BuildAuthedUrlParams): URL {
    const config = toRecord(params.config);
    const baseUrl = `${config.serverUrl || ''}`.trim();
    const token = `${config.clientToken || ''}`.trim();

    const url = new URL(params.pathname, baseUrl);

    if (token) {
      url.searchParams.set('token', token);
    }

    const options = params.options || {};
    for (const [key, value] of Object.entries(options)) {
      if (value === null || value === undefined || value === '') {
        continue;
      }

      url.searchParams.set(key, `${value}`);
    }

    return url;
  }

  function applyPreviewVolume(params: ApplyPreviewVolumeParams): void {
    if (!(params.root instanceof Element)) {
      return;
    }

    const gain = toPerceptualGain(params.linearVolume, params.gamma);
    const mediaElements = params.root.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      if (element instanceof HTMLMediaElement) {
        element.volume = gain;
        element.muted = false;
      }
    });
  }

  return {
    buildAuthedUrl,
    applyPreviewVolume,
  };
}
