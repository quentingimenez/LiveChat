interface OverlayMediaPayloadLike {
  kind?: unknown;
}

interface OverlayPlayPayloadLike {
  media?: OverlayMediaPayloadLike;
  durationSec?: unknown;
  jobId?: unknown;
}

interface ApplyOverlayInfoOptions {
  showAuthorInline: boolean;
  attachTextToMedia: boolean;
}

interface ApplyMediaTextOptions {
  enabled: boolean;
}

interface ReportErrorPayload {
  jobId: string;
  code: string;
  message: string;
}

interface RunPlayFlowParams {
  payload: OverlayPlayPayloadLike | null | undefined;
  clearOverlay: () => void;
  startPlaybackSession: (jobId: unknown) => void;
  applyOverlayInfo: (payload: OverlayPlayPayloadLike | null | undefined, options: ApplyOverlayInfoOptions) => void;
  renderTweetCard: (payload: OverlayPlayPayloadLike | null | undefined) => boolean;
  renderMedia: (payload: OverlayPlayPayloadLike | null | undefined) => Promise<void>;
  applyMediaHeaderAuthor: (payload: OverlayPlayPayloadLike | null | undefined, options: ApplyMediaTextOptions) => void;
  applyMediaFooterText: (payload: OverlayPlayPayloadLike | null | undefined, options: ApplyMediaTextOptions) => void;
  logRenderError: (message: string, error: unknown) => void;
  reportError: (payload: ReportErrorPayload) => void;
  startCountdown: (durationSec: unknown, options: { autoClear?: boolean }) => void;
  setResetTimer: (timer: number | null) => void;
  setTimeout: (callback: () => void, delayMs: number) => number;
  clearCountdown: () => void;
}

export interface OverlayLegacyPlayFlowUtils {
  runPlayFlow(params: RunPlayFlowParams): Promise<void>;
}

function resolveJobId(jobId: unknown): string {
  if (typeof jobId !== 'string' || jobId.trim() === '') {
    return 'unknown-job';
  }

  return jobId;
}

function hasStandaloneMedia(payload: OverlayPlayPayloadLike | null | undefined): boolean {
  return Boolean(payload?.media);
}

function shouldAutoClearByTimer(payload: OverlayPlayPayloadLike | null | undefined, hasMedia: boolean): boolean {
  if (!hasMedia) {
    return true;
  }

  return payload?.media?.kind === 'image';
}

export function createOverlayLegacyPlayFlowUtils(): OverlayLegacyPlayFlowUtils {
  async function runPlayFlow(params: RunPlayFlowParams): Promise<void> {
    params.clearOverlay();
    params.startPlaybackSession(params.payload?.jobId || null);

    const hasMedia = hasStandaloneMedia(params.payload);
    if (!hasMedia) {
      params.applyOverlayInfo(params.payload, {
        showAuthorInline: true,
        attachTextToMedia: false,
      });
    }

    const autoClearByTimer = shouldAutoClearByTimer(params.payload, hasMedia);

    try {
      if (!params.renderTweetCard(params.payload)) {
        await params.renderMedia(params.payload);
      }

      params.applyMediaHeaderAuthor(params.payload, {
        enabled: hasMedia,
      });
      params.applyMediaFooterText(params.payload, {
        enabled: hasMedia,
      });

      if (hasMedia) {
        params.applyOverlayInfo(params.payload, {
          showAuthorInline: false,
          attachTextToMedia: true,
        });
      }
    } catch (error) {
      params.logRenderError('Media render failed:', error);
      params.reportError({
        jobId: resolveJobId(params.payload?.jobId),
        code: 'render_failed',
        message: error instanceof Error && typeof error.message === 'string' ? error.message : 'unknown_render_error',
      });
      params.clearOverlay();
      return;
    }

    if (params.payload?.durationSec) {
      params.startCountdown(params.payload.durationSec, { autoClear: autoClearByTimer });

      if (autoClearByTimer) {
        const resetTimer = params.setTimeout(() => {
          params.clearOverlay();
        }, Number(params.payload.durationSec) * 1000 + 100);
        params.setResetTimer(resetTimer);
      }
      return;
    }

    params.clearCountdown();
  }

  return {
    runPlayFlow,
  };
}
