interface AttachMediaElementParams {
  targetContainer: unknown;
  element: unknown;
  mediaKind: unknown;
  onMediaReady: () => void;
}

interface ApplyMediaVolumeParams {
  root: unknown;
  perceptualGain: number;
}

export interface OverlayLegacyMediaDomUtils {
  createMediaElement(kind: unknown): HTMLImageElement | HTMLAudioElement | HTMLVideoElement;
  attachMediaElement(params: AttachMediaElementParams): boolean;
  applyMediaVolume(params: ApplyMediaVolumeParams): void;
  buildAuthorizedMediaUrl(rawUrl: unknown, clientToken: unknown): string;
}

function toMediaKind(kind: unknown): 'image' | 'audio' | 'video' {
  const normalized = `${kind || ''}`.trim().toLowerCase();

  if (normalized === 'image' || normalized === 'audio' || normalized === 'video') {
    return normalized;
  }

  return 'video';
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

export function createOverlayLegacyMediaDomUtils(): OverlayLegacyMediaDomUtils {
  function createMediaElement(kind: unknown): HTMLImageElement | HTMLAudioElement | HTMLVideoElement {
    const mediaKind = toMediaKind(kind);

    if (mediaKind === 'image') {
      return document.createElement('img');
    }

    if (mediaKind === 'audio') {
      const audio = document.createElement('audio');
      audio.autoplay = true;
      audio.controls = false;
      audio.preload = 'auto';
      return audio;
    }

    const video = document.createElement('video');
    video.autoplay = true;
    video.controls = false;
    video.playsInline = true;
    video.preload = 'auto';
    return video;
  }

  function attachMediaElement(params: AttachMediaElementParams): boolean {
    const targetContainer = params.targetContainer;
    const element = params.element;
    if (!(targetContainer instanceof HTMLElement)) {
      return false;
    }

    if (!(element instanceof HTMLElement)) {
      return false;
    }

    const mediaKind = toMediaKind(params.mediaKind);
    targetContainer.appendChild(element);

    if (mediaKind === 'image') {
      element.addEventListener(
        'load',
        () => {
          params.onMediaReady();
        },
        { once: true }
      );
      return true;
    }

    element.addEventListener('loadedmetadata', () => {
      params.onMediaReady();
    });
    return true;
  }

  function applyMediaVolume(params: ApplyMediaVolumeParams): void {
    const root = params.root;
    if (!(root instanceof HTMLElement)) {
      return;
    }

    const mediaElements = root.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      if (!(element instanceof HTMLMediaElement)) {
        return;
      }

      const forceMuted = element.dataset?.forceMuted === '1';
      if (forceMuted) {
        element.volume = 0;
        element.muted = true;
        return;
      }

      const gain = isFiniteNumber(params.perceptualGain) ? params.perceptualGain : 1;
      element.volume = gain;
      element.muted = false;
    });
  }

  function buildAuthorizedMediaUrl(rawUrl: unknown, clientToken: unknown): string {
    const mediaUrl = new URL(`${rawUrl || ''}`);
    mediaUrl.searchParams.set('token', `${clientToken || ''}`);
    return mediaUrl.toString();
  }

  return {
    createMediaElement,
    attachMediaElement,
    applyMediaVolume,
    buildAuthorizedMediaUrl
  };
}
