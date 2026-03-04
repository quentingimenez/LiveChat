interface OverlayMediaPayloadLike {
  url?: unknown;
  kind?: unknown;
}

interface OverlayPlayPayloadLike {
  media?: OverlayMediaPayloadLike;
  jobId?: unknown;
}

interface AttachMediaElementParamsLike {
  targetContainer: unknown;
  element: unknown;
  mediaKind: unknown;
  onMediaReady: () => void;
}

interface AttachMediaDiagnosticsParams {
  mediaUrl: string;
  mediaKind: unknown;
  jobId: string;
}

interface RenderMediaParams {
  payload: OverlayPlayPayloadLike | null | undefined;
  setActiveObjectUrl: (nextObjectUrl: string | null) => void;
  buildAuthorizedMediaUrl: (rawUrl: unknown) => string;
  createMediaElement: (kind: unknown) => HTMLImageElement | HTMLAudioElement | HTMLVideoElement;
  attachMediaDiagnostics: (
    element: HTMLImageElement | HTMLAudioElement | HTMLVideoElement,
    params: AttachMediaDiagnosticsParams,
  ) => void;
  ensureMediaFrame: () => HTMLElement | null;
  mediaLayer: unknown;
  attachMediaElement: (params: AttachMediaElementParamsLike) => boolean;
  scheduleMediaHeaderLayout: () => void;
  applyVolume: () => void;
  setActivePlayableElement: (element: HTMLMediaElement | null) => void;
  applyMediaStartOffset: (element: HTMLMediaElement, startOffsetSec: number | null) => void;
  resolveStartOffsetSec: (mediaPayload: unknown) => number | null;
  resumeCountdown: () => void;
  pauseCountdown: () => void;
  setPlaybackState: (state: string) => void;
  clearOverlay: () => void;
  logAutoplayError: (message: string, error: unknown) => void;
}

export interface OverlayLegacyMediaRenderUtils {
  renderMedia(params: RenderMediaParams): Promise<void>;
}

function resolveJobId(jobId: unknown): string {
  if (typeof jobId !== 'string' || jobId.trim() === '') {
    return 'unknown-job';
  }

  return jobId;
}

export function createOverlayLegacyMediaRenderUtils(): OverlayLegacyMediaRenderUtils {
  async function renderMedia(params: RenderMediaParams): Promise<void> {
    const mediaPayload = params.payload?.media;
    if (!mediaPayload) {
      return;
    }

    const mediaUrl = params.buildAuthorizedMediaUrl(mediaPayload.url);
    params.setActiveObjectUrl(null);

    const element = params.createMediaElement(mediaPayload.kind);
    params.attachMediaDiagnostics(element, {
      mediaUrl,
      mediaKind: mediaPayload.kind,
      jobId: resolveJobId(params.payload?.jobId),
    });
    element.src = mediaUrl;

    const targetContainer = params.ensureMediaFrame() || params.mediaLayer;
    if (!(targetContainer instanceof HTMLElement)) {
      return;
    }

    const attached = params.attachMediaElement({
      targetContainer,
      element,
      mediaKind: mediaPayload.kind,
      onMediaReady: params.scheduleMediaHeaderLayout,
    });
    if (!attached) {
      return;
    }

    params.scheduleMediaHeaderLayout();
    params.applyVolume();

    if (mediaPayload.kind === 'image') {
      return;
    }

    const playableElement = element as HTMLMediaElement;
    params.setActivePlayableElement(playableElement);
    params.applyMediaStartOffset(playableElement, params.resolveStartOffsetSec(mediaPayload));

    playableElement.addEventListener('playing', () => {
      params.resumeCountdown();
      params.setPlaybackState('playing');
    });

    playableElement.addEventListener('pause', () => {
      if (playableElement.ended) {
        return;
      }

      params.pauseCountdown();
      params.setPlaybackState('paused');
    });

    try {
      await playableElement.play();
    } catch (error) {
      params.logAutoplayError('Autoplay failed:', error);
      params.pauseCountdown();
      params.setPlaybackState('paused');
    }

    playableElement.addEventListener(
      'ended',
      () => {
        params.clearOverlay();
      },
      { once: true },
    );
  }

  return {
    renderMedia,
  };
}
